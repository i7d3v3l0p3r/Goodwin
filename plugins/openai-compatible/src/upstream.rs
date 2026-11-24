use std::{io::Read, time::Duration};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::provider::{
    Capability, ChatRequest, Completion, MAX_CHAT_BYTES, MAX_RECORDING_BYTES, MAX_SPEECH_BYTES,
    MAX_SPEECH_CHUNK_BYTES, Model, ModelCatalog, ProviderHttpError, SpeechAudio, SpeechRequest,
    Transcription, TranscriptionRequest,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Config;

mod chat_stream;
mod transport;
use transport::{Client, Method, RequestBuilder, Response, send};

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    OpenAi,
    OpenRouter,
}

pub struct Upstream {
    client: Client,
    protocol: Protocol,
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<Completion>,
}

#[derive(Deserialize)]
struct UpstreamCatalog {
    data: Vec<UpstreamModel>,
}

#[derive(Deserialize)]
struct UpstreamModel {
    id: String,
    name: Option<String>,
    #[serde(default)]
    output_modalities: Vec<Value>,
}

impl Upstream {
    pub fn new(protocol: Protocol) -> Result<Self> {
        Ok(Self {
            client: transport::client()?,
            protocol,
        })
    }

    pub fn with_protocol(&self, protocol: Protocol) -> Self {
        Self {
            client: self.client.clone(),
            protocol,
        }
    }

    pub fn transcribe(
        &self,
        config: &Config,
        request: TranscriptionRequest,
    ) -> Result<Transcription> {
        request.selection.validate()?;
        let model = request
            .selection
            .model
            .as_deref()
            .unwrap_or(&config.transcription_model);
        ensure!(!model.is_empty(), "Configure a transcription model");
        ensure!(
            !request.wav_base64.is_empty()
                && request.wav_base64.len() <= MAX_RECORDING_BYTES.div_ceil(3) * 4,
            "Invalid recording size"
        );
        let post = self.inference(config, &["audio", "transcriptions"])?;
        let post = match self.protocol {
            Protocol::OpenRouter => {
                let mut body = request.selection.parameters;
                body.insert("model".into(), json!(model));
                body.insert("language".into(), json!(request.language));
                body.insert(
                    "input_audio".into(),
                    json!({"data": request.wav_base64, "format": "wav"}),
                );
                post.json(&body)
            }
            Protocol::OpenAi => {
                let wav = STANDARD.decode(request.wav_base64)?;
                ensure!(wav.len() <= MAX_RECORDING_BYTES, "Invalid recording size");
                let mut fields = vec![
                    ("model".into(), model.to_owned()),
                    ("language".into(), request.language),
                    ("response_format".into(), "json".into()),
                ];
                for (name, value) in request.selection.parameters {
                    let text = match value {
                        Value::String(text) => text,
                        value => value.to_string(),
                    };
                    fields.push((name, text));
                }
                transport::wave(post, wav, fields)?
            }
        };
        Ok(serde_json::from_slice(&read(send(post)?, 64_000)?)?)
    }

    pub fn complete(&self, config: &Config, request: ChatRequest) -> Result<Completion> {
        let response = send(self.chat_request(config, &request, false)?)?;
        read_completion(response)
    }

    pub fn complete_stream(
        &self,
        config: &Config,
        request: ChatRequest,
        emit: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            request.tools.is_empty(),
            "Chat text streaming does not support tools"
        );
        let response = match send(self.chat_request(config, &request, true)?) {
            Err(error)
                if self.protocol == Protocol::OpenAi
                    && error
                        .downcast_ref::<ProviderHttpError>()
                        .is_some_and(|status| matches!(status.0, 400 | 406 | 415 | 422 | 501)) =>
            {
                return emit_completion(&self.complete(config, request)?, emit);
            }
            result => result?,
        };
        let is_event_stream = transport::content_type(&response).is_some_and(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("text/event-stream")
        });
        if is_event_stream {
            return chat_stream::read(response, emit);
        }
        emit_completion(&read_completion(response)?, emit)
    }

    fn chat_request(
        &self,
        config: &Config,
        request: &ChatRequest,
        stream: bool,
    ) -> Result<RequestBuilder> {
        let selection = &request.selection;
        selection.validate()?;
        let model = selection.model.as_deref().unwrap_or(&config.chat_model);
        ensure!(!model.is_empty(), "Configure a chat model");
        let require_parameters = self.protocol == Protocol::OpenRouter && !request.tools.is_empty();
        let mut request = serde_json::to_value(request)?;
        let fields = request
            .as_object_mut()
            .context("Expected a chat request object")?;
        fields.remove("selection");
        fields.extend(selection.parameters.clone());
        request["model"] = json!(model);
        if require_parameters {
            request["provider"] = json!({"require_parameters": true});
        }
        let mut post = self.inference(config, &["chat", "completions"])?;
        if stream {
            request["stream"] = json!(true);
            post = transport::accept(post, "text/event-stream");
        }
        Ok(post.json(&request))
    }

    pub fn synthesize(&self, config: &Config, request: SpeechRequest) -> Result<SpeechAudio> {
        let response = send(self.speech_request(config, request)?)?;
        let (sample_rate, channels) = speech_format(&response)?;
        let data = read(response, MAX_SPEECH_BYTES)?;
        ensure!(
            !data.is_empty() && data.len().is_multiple_of(2 * usize::from(channels)),
            "Invalid PCM length"
        );
        Ok(SpeechAudio {
            pcm_base64: STANDARD.encode(data),
            sample_rate,
            channels,
        })
    }

    pub fn synthesize_stream(
        &self,
        config: &Config,
        request: SpeechRequest,
        emit: &mut dyn FnMut(SpeechAudio) -> Result<()>,
    ) -> Result<()> {
        let mut response = send(
            transport::accept(self.speech_request(config, request)?, "audio/pcm")
                .timeout(Duration::from_secs(180)),
        )?;
        let (sample_rate, channels) = speech_format(&response)?;
        let frame_bytes = 2 * usize::from(channels);
        let mut buffer = [0_u8; MAX_SPEECH_CHUNK_BYTES];
        let mut pending = 0;
        let mut total = 0;
        loop {
            let count = response
                .read(&mut buffer[pending..])
                .context("Read provider PCM")?;
            if count == 0 {
                ensure!(total != 0 && pending == 0, "Invalid PCM length");
                return Ok(());
            }
            total += count;
            ensure!(
                total <= MAX_SPEECH_BYTES,
                "Provider response exceeds size limit"
            );
            let available = pending + count;
            let aligned = available / frame_bytes * frame_bytes;
            if aligned != 0 {
                emit(SpeechAudio {
                    pcm_base64: STANDARD.encode(&buffer[..aligned]),
                    sample_rate,
                    channels,
                })?;
            }
            buffer.copy_within(aligned..available, 0);
            pending = available - aligned;
        }
    }

    fn speech_request(&self, config: &Config, request: SpeechRequest) -> Result<RequestBuilder> {
        request.selection.validate()?;
        let model = request
            .selection
            .model
            .as_deref()
            .unwrap_or(&config.speech_model);
        ensure!(!model.is_empty(), "Configure a speech model");
        let mut body = request.selection.parameters;
        if model == config.speech_model && !config.voice.is_empty() {
            body.entry("voice").or_insert_with(|| json!(config.voice));
        }
        body.insert("model".into(), json!(model));
        body.insert("input".into(), json!(request.text));
        body.insert("response_format".into(), json!("pcm"));
        Ok(self.inference(config, &["audio", "speech"])?.json(&body))
    }

    pub fn models(&self, config: &Config, capability: Capability) -> ModelCatalog {
        let default = match capability {
            Capability::Transcription => &config.transcription_model,
            Capability::Chat => &config.chat_model,
            Capability::Speech => &config.speech_model,
        };
        let catalog = self.fetch_models(config, capability);
        let available = catalog.is_ok();
        let mut models = catalog.unwrap_or_default();
        if !default.is_empty() && !models.iter().any(|model| model.id == *default) {
            models.insert(
                0,
                Model {
                    id: default.clone(),
                    name: None,
                    voices: Vec::new(),
                },
            );
        }
        ModelCatalog {
            models,
            default: (!default.is_empty()).then(|| default.clone()),
            available,
        }
    }

    fn fetch_models(&self, config: &Config, capability: Capability) -> Result<Vec<Model>> {
        let openrouter = self.protocol == Protocol::OpenRouter;
        let speech = openrouter && capability == Capability::Speech;
        let mut request = self
            .request(
                config,
                Method::Get,
                if speech {
                    &["audio", "speech", "models"]
                } else {
                    &["models"]
                },
            )?
            .timeout(Duration::from_secs(3));
        if openrouter && !speech {
            let endpoint = match capability {
                Capability::Transcription => "/api/v1/audio/transcriptions",
                _ => "/api/v1/chat/completions",
            };
            request = request.query(&[("endpoints", endpoint)]);
        }
        let catalog: UpstreamCatalog = serde_json::from_slice(&read(send(request)?, 8_000_000)?)?;
        let mut ids = std::collections::HashSet::new();
        let mut bytes = 0;
        Ok(catalog
            .data
            .into_iter()
            .filter_map(|model| {
                if model.id.trim() != model.id
                    || model.id.is_empty()
                    || model.id.len() > 256
                    || model.id.chars().any(char::is_control)
                    || !ids.insert(model.id.clone())
                {
                    return None;
                }
                let speech_options = model
                    .output_modalities
                    .iter()
                    .find(|item| item["type"] == "speech");
                let parameters = speech_options.and_then(|item| item.get("supported_parameters"));
                if parameters
                    .and_then(|params| params["response_format"]["values"].as_array())
                    .is_some_and(|formats| !formats.iter().any(|format| format == "pcm"))
                {
                    return None;
                }
                let voices = parameters
                    .and_then(|params| params["voice"]["values"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|voice| voice.len() <= 100 && !voice.chars().any(char::is_control))
                    .take(100)
                    .map(str::to_owned)
                    .collect();
                Some(Model {
                    id: model.id,
                    name: model
                        .name
                        .filter(|name| name.len() <= 256 && !name.chars().any(char::is_control)),
                    voices,
                })
            })
            .take(750)
            .take_while(|model| {
                bytes += serde_json::to_vec(model).map_or(512 * 1024, |value| value.len() + 1);
                bytes < 450 * 1024
            })
            .collect())
    }

    fn inference(&self, config: &Config, endpoint: &[&str]) -> Result<RequestBuilder> {
        #[cfg(not(target_arch = "wasm32"))]
        ensure!(
            self.protocol != Protocol::OpenRouter || !config.key.trim().is_empty(),
            "Configure an OpenRouter API key in the plugin settings"
        );
        self.request(config, Method::Post, endpoint)
    }

    fn request(
        &self,
        config: &Config,
        method: Method,
        endpoint: &[&str],
    ) -> Result<RequestBuilder> {
        transport::request(&self.client, config, method, endpoint)
    }
}

fn read_completion(response: Response) -> Result<Completion> {
    let completion: ChatCompletion = serde_json::from_slice(&read(response, MAX_CHAT_BYTES)?)?;
    completion
        .choices
        .into_iter()
        .next()
        .context("Server returned no completion")
}

fn emit_completion(
    completion: &Completion,
    emit: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let text = completion.message["content"]
        .as_str()
        .context("Missing provider answer")?;
    chat_stream::emit_text(text, emit)
}

fn read(response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .context("Read provider response")?;
    ensure!(body.len() <= limit, "Provider response exceeds size limit");
    Ok(body)
}

fn pcm_format(content_type: &str) -> Result<(u32, u16)> {
    let mut parts = content_type.split(';').map(str::trim);
    ensure!(
        parts
            .next()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("audio/pcm")
                || mime.eq_ignore_ascii_case("application/octet-stream")),
        "Provider did not return raw PCM"
    );
    let mut rate = 24_000;
    let mut channels = 1;
    for (name, value) in parts.filter_map(|part| part.split_once('=')) {
        let value = value.trim().trim_matches('"');
        match name.trim().to_ascii_lowercase().as_str() {
            "rate" => rate = value.parse()?,
            "channels" => channels = value.parse()?,
            _ => {}
        }
    }
    ensure!(
        (8_000..=96_000).contains(&rate) && (1..=2).contains(&channels),
        "Invalid PCM format"
    );
    Ok((rate, channels))
}

fn speech_format(response: &Response) -> Result<(u32, u16)> {
    pcm_format(transport::content_type(response).unwrap_or_default())
}
