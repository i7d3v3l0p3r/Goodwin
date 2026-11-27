use std::{io::Read, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Context, Result, ensure};
use goodwin_plugin::{
    json_codec,
    provider::{
        Capability, ChatCompletion, ChatPayload, ChatRequest, ChatRole, Completion, MAX_CHAT_BYTES,
        MAX_RECORDING_BYTES, MAX_SPEECH_BYTES, MAX_SPEECH_CHUNK_BYTES, Model, ModelCatalog,
        PcmFormat, SpeechAudio, SpeechRequest, Transcription, TranscriptionRequest,
    },
};
use serde::{Deserialize, Serialize};

use super::Config;

mod catalog;
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

#[derive(Serialize)]
struct TranscriptionBody<'a> {
    model: &'a str,
    language: &'a str,
    input_audio: InputAudio<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
}

#[derive(Serialize)]
struct InputAudio<'a> {
    data: &'a str,
    format: &'static str,
}

#[derive(Serialize)]
struct SpeechBody<'a> {
    model: &'a str,
    input: &'a str,
    response_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
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
        let parameters = &request.selection.parameters;
        let post = match self.protocol {
            Protocol::OpenRouter => {
                let body = TranscriptionBody {
                    model,
                    language: &request.language,
                    input_audio: InputAudio {
                        data: &request.wav_base64,
                        format: "wav",
                    },
                    temperature: parameters.temperature,
                    prompt: parameters.prompt.as_deref(),
                };
                let capacity = request.wav_base64.len()
                    + 6 * (model.len() + request.language.len() + body.prompt.map_or(0, str::len))
                    + 256;
                transport::json(post, json_codec::to_vec_with_capacity(&body, capacity)?)
            }
            Protocol::OpenAi => {
                let wav = STANDARD.decode(request.wav_base64)?;
                ensure!(wav.len() <= MAX_RECORDING_BYTES, "Invalid recording size");
                let mut fields = vec![
                    ("model".into(), model.to_owned()),
                    ("language".into(), request.language),
                    ("response_format".into(), "json".into()),
                ];
                if let Some(temperature) = parameters.temperature {
                    fields.push(("temperature".into(), temperature.to_string()));
                }
                if let Some(prompt) = &parameters.prompt {
                    fields.push(("prompt".into(), prompt.clone()));
                }
                transport::wave(post, wav, fields)?
            }
        };
        Ok(goodwin_plugin::json_codec::from_slice(&read(
            send(post)?,
            64_000,
        )?)?)
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
            Err(transport::SendError::Http(status))
                if self.protocol == Protocol::OpenAi
                    && matches!(status.0, 400 | 406 | 415 | 422 | 501) =>
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
        let body = ChatPayload::new(
            model,
            request,
            stream,
            self.protocol == Protocol::OpenRouter,
        );
        let mut post = self.inference(config, &["chat", "completions"])?;
        if stream {
            post = transport::accept(post, "text/event-stream");
        }
        Ok(transport::json(post, json_codec::to_vec(&body)?))
    }

    pub fn synthesize(&self, config: &Config, request: SpeechRequest) -> Result<SpeechAudio> {
        let response = send(self.speech_request(config, request)?)?;
        let format = speech_format(&response)?;
        let data = read(response, MAX_SPEECH_BYTES)?;
        ensure!(
            !data.is_empty() && data.len().is_multiple_of(format.frame_bytes()),
            "Invalid PCM length"
        );
        Ok(SpeechAudio {
            pcm_base64: STANDARD.encode(data),
            sample_rate: format.sample_rate,
            channels: format.channels,
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
        let format = speech_format(&response)?;
        let frame_bytes = format.frame_bytes();
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
                    sample_rate: format.sample_rate,
                    channels: format.channels,
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
        let parameters = &request.selection.parameters;
        let body = SpeechBody {
            model,
            input: &request.text,
            response_format: "pcm",
            voice: parameters.voice.as_deref().or_else(|| {
                (model == config.speech_model && !config.voice.is_empty())
                    .then_some(config.voice.as_str())
            }),
            speed: parameters.speed,
            instructions: parameters.instructions.as_deref(),
        };
        Ok(transport::json(
            self.inference(config, &["audio", "speech"])?,
            json_codec::to_vec(&body)?,
        ))
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
            .timeout(Duration::from_secs(15));
        if openrouter && !speech {
            let endpoint = match capability {
                Capability::Transcription => "/api/v1/audio/transcriptions",
                _ => "/api/v1/chat/completions",
            };
            request = request.query(&[("endpoints", endpoint)]);
        }
        catalog::read(send(request)?)
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
    let completion: ChatCompletion = json_codec::from_slice(&read(response, MAX_CHAT_BYTES)?)?;
    let first = completion
        .choices
        .into_iter()
        .next()
        .context("Server returned no completion")?;
    ensure!(
        first.message.role == ChatRole::Assistant,
        "Invalid completion role"
    );
    Ok(first)
}

fn emit_completion(
    completion: &Completion,
    emit: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let text = completion
        .message
        .content
        .as_deref()
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

fn speech_format(response: &Response) -> Result<PcmFormat> {
    transport::content_type(response)
        .and_then(PcmFormat::from_content_type)
        .context("Provider did not return valid signed 16-bit little-endian PCM")
}
