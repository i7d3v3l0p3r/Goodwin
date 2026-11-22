use std::{collections::BTreeMap, time::Duration};

use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Response, header::CONTENT_TYPE, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::wav::PcmAudio;

#[derive(Clone, Serialize, Deserialize)]
pub struct Models {
    pub transcription: String,
    pub chat: String,
    pub speech: String,
    pub voice: String,
}

impl Default for Models {
    fn default() -> Self {
        Self {
            transcription: "openai/whisper-large-v3".into(),
            chat: "openai/gpt-4.1-mini".into(),
            speech: "microsoft/mai-voice-2.1-flash".into(),
            voice: "ru-RU-Lev:MAI-Voice-2.1-Flash".into(),
        }
    }
}

impl Models {
    pub fn for_file_request() -> Self {
        let mut models = Self::default();
        for (name, model) in [
            ("GOODWIN_STT_MODEL", &mut models.transcription),
            ("GOODWIN_CHAT_MODEL", &mut models.chat),
            ("GOODWIN_TTS_MODEL", &mut models.speech),
            ("GOODWIN_TTS_VOICE", &mut models.voice),
        ] {
            if let Ok(value) = std::env::var(name)
                && !value.trim().is_empty()
            {
                *model = value.trim().to_owned();
            }
        }
        models
    }
}

pub struct OpenRouter {
    client: Client,
    base_url: String,
}

#[derive(Deserialize)]
struct Transcription {
    text: String,
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Value,
}

#[derive(Deserialize)]
pub(crate) struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

#[derive(Deserialize)]
pub(crate) struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub(crate) struct CompletionMessage(Value);

impl CompletionMessage {
    pub(crate) fn text(&self) -> Result<String> {
        let content = self.0["content"].as_str().unwrap_or_default().trim();
        ensure!(!content.is_empty(), "Empty OpenRouter answer");
        Ok(content.to_owned())
    }

    pub(crate) fn tool_calls(&self) -> Result<Vec<ToolCall>> {
        match self.0.get("tool_calls") {
            Some(calls) if !calls.is_null() => Ok(serde_json::from_value(calls.clone())?),
            _ => Ok(Vec::new()),
        }
    }
}

pub(crate) struct ToolConversation {
    messages: Vec<Value>,
    definitions: Vec<Value>,
}

impl ToolConversation {
    pub(crate) fn new<'a>(
        instructions: &str,
        text: &str,
        tools: impl Iterator<Item = (&'a str, &'a str, &'a Value)>,
        context: BTreeMap<String, Value>,
    ) -> Self {
        let definitions = tools.map(|(name, description, parameters)| json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters }
        })).collect();
        let mut messages = vec![json!({"role": "system", "content": instructions})];
        if !context.is_empty() {
            messages.push(
                json!({"role": "user", "content": json!({"pluginContext": context}).to_string()}),
            );
        }
        messages.push(json!({"role": "user", "content": text}));
        Self {
            messages,
            definitions,
        }
    }

    pub(crate) fn push_completion(&mut self, message: CompletionMessage) {
        self.messages.push(message.0);
    }

    pub(crate) fn push_tool_result(&mut self, id: &str, result: &Value) {
        self.messages.push(json!({
            "role": "tool", "tool_call_id": id, "content": result.to_string()
        }));
    }
}

impl OpenRouter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .redirect(Policy::none())
                .build()?,
            base_url: "https://openrouter.ai/api/v1".into(),
        })
    }

    #[cfg(test)]
    pub fn pointed_at(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    pub async fn transcribe(&self, key: &str, models: &Models, wav: &[u8]) -> Result<String> {
        let response = self
            .post(
                key,
                "audio/transcriptions",
                json!({
                    "model": models.transcription,
                    "language": "ru",
                    "input_audio": {"data": STANDARD.encode(wav), "format": "wav"}
                }),
            )
            .await?;
        let transcript: Transcription =
            serde_json::from_slice(&read_body(response, 128_000).await?)?;
        Ok(transcript.text.trim().to_owned())
    }

    pub async fn reply<'a>(
        &self,
        key: &str,
        models: &Models,
        text: &str,
        instructions: &str,
        history: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<String> {
        let mut messages = vec![json!({"role": "system", "content": instructions})];
        messages.extend(history.into_iter().map(|(role, content)| {
            json!({
                "role": role,
                "content": content
            })
        }));
        messages.push(json!({"role": "user", "content": text}));
        let message = self
            .complete(
                key,
                json!({ "model": models.chat, "messages": messages, "max_completion_tokens": 256 }),
            )
            .await?;
        message.text()
    }

    pub(crate) async fn complete_with_tools(
        &self,
        key: &str,
        models: &Models,
        conversation: &ToolConversation,
        can_call: bool,
    ) -> Result<CompletionMessage> {
        self.complete(
            key,
            json!({
                "model": models.chat, "messages": conversation.messages, "max_completion_tokens": 256,
                "tools": conversation.definitions, "tool_choice": if can_call { "auto" } else { "none" },
                "provider": { "require_parameters": true }
            }),
        )
        .await
    }

    async fn complete(&self, key: &str, body: Value) -> Result<CompletionMessage> {
        let response = self.post(key, "chat/completions", body).await?;
        let completion: ChatCompletion =
            serde_json::from_slice(&read_body(response, 256_000).await?)?;
        completion
            .choices
            .into_iter()
            .next()
            .map(|choice| CompletionMessage(choice.message))
            .ok_or_else(|| anyhow::anyhow!("OpenRouter returned no completion"))
    }

    pub async fn synthesize(&self, key: &str, models: &Models, answer: &str) -> Result<PcmAudio> {
        let response = self
            .post(
                key,
                "audio/speech",
                json!({
                    "model": models.speech,
                    "voice": models.voice,
                    "input": answer,
                    "response_format": "pcm"
                }),
            )
            .await?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let (sample_rate, channels) = pcm_format(content_type)?;
        let data = read_body(response, 8_000_000).await?;
        ensure!(
            !data.is_empty() && data.len() % (2 * usize::from(channels)) == 0,
            "Invalid PCM length"
        );
        Ok(PcmAudio {
            data,
            sample_rate,
            channels,
        })
    }

    async fn post(&self, key: &str, endpoint: &str, data: Value) -> Result<Response> {
        ensure!(!key.is_empty(), "OpenRouter is not configured");
        let response = self
            .client
            .post(format!("{}/{endpoint}", self.base_url))
            .bearer_auth(key)
            .json(&data)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            bail!("OpenRouter returned HTTP {status}");
        }
        Ok(response)
    }
}

async fn read_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= limit - body.len(),
            "OpenRouter response exceeds size limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn pcm_format(content_type: &str) -> Result<(u32, u16)> {
    let mut parameters = content_type.split(';').map(str::trim);
    ensure!(
        parameters
            .next()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("audio/pcm")),
        "OpenRouter returned an unexpected audio format"
    );
    let mut rate = 24_000;
    let mut channels = 1;
    for (key, value) in parameters.filter_map(|parameter| parameter.split_once('=')) {
        let value = value.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
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
