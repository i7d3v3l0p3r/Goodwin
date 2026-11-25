use goodwin_error::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::panel_core::Message;

use super::ProviderSettings;

pub(crate) const SPEECH_HZ: u32 = 24_000;

pub(crate) trait HttpTransport {
    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[&[u8]],
        limit: usize,
        chunk: impl FnMut(Option<&str>, &[u8]) -> Result<()>,
    ) -> Result<()>;
}

const JSON_LIMIT: usize = 32 * 1024;
const SPEECH_LIMIT: usize = SPEECH_HZ as usize * 2 * 30;

#[derive(Clone, Copy)]
pub(crate) struct Provider<'a, T> {
    settings: &'a ProviderSettings,
    transport: T,
}

impl<'a, T: HttpTransport> Provider<'a, T> {
    pub(crate) fn new(settings: &'a ProviderSettings, transport: T) -> Self {
        Self {
            settings,
            transport,
        }
    }

    pub(crate) fn transcribe(&self, wav: &[u8]) -> Result<String> {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|_| goodwin_error::report!("Generate recording boundary"))?;
        let boundary = format!("goodwin-{:016x}", u64::from_le_bytes(random));
        let model = &self.settings.stt_model;
        let content_type = format!("multipart/form-data; boundary={boundary}");
        let prefix = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\n\
             Content-Type: audio/wav\r\n\r\n"
        );
        let suffix = format!("\r\n--{boundary}--\r\n");
        let reply = self.response(
            "/audio/transcriptions",
            &content_type,
            &[prefix.as_bytes(), wav, suffix.as_bytes()],
            JSON_LIMIT,
        )?;
        #[derive(Deserialize)]
        struct Response {
            text: String,
        }
        Ok(goodwin_json::from_slice::<Response>(&reply)?.text)
    }

    pub(crate) fn reply(&self, user: &str, instructions: &str) -> Result<String> {
        self.complete(user, instructions, &[])
    }

    #[cfg(feature = "wasm-plugins")]
    pub(crate) fn complete_tools(
        &self,
        request: goodwin_plugin::provider::ChatRequest,
    ) -> Result<crate::voice_core::tools::CompletionMessage> {
        use goodwin_plugin::provider::{ChatCompletion, ChatPayload, ChatRole};

        let body = goodwin_json::to_vec(&ChatPayload::new(
            &self.settings.chat_model,
            &request,
            false,
            false,
        ))?;
        let reply = self.response(
            "/chat/completions",
            "application/json",
            &[&body],
            JSON_LIMIT,
        )?;
        let completion = goodwin_json::from_slice::<ChatCompletion>(&reply)?
            .choices
            .into_iter()
            .next()
            .context("Chat returned no message")?;
        ensure!(
            completion.message.role == ChatRole::Assistant,
            "Invalid completion role"
        );
        Ok(crate::voice_core::tools::CompletionMessage(
            completion.message,
        ))
    }

    pub(crate) fn chat(
        &self,
        user: &str,
        history: &[Message],
        instructions: &str,
    ) -> Result<String> {
        self.complete(user, instructions, history)
    }

    fn complete(&self, user: &str, instructions: &str, history: &[Message]) -> Result<String> {
        #[derive(Serialize)]
        struct ChatMessage<'a> {
            role: &'static str,
            content: &'a str,
        }
        #[derive(Serialize)]
        struct ChatInput<'a> {
            model: &'a str,
            messages: Vec<ChatMessage<'a>>,
        }
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(ChatMessage {
            role: "system",
            content: instructions,
        });
        messages.extend(history.iter().rev().map(|message| ChatMessage {
            role: message.role,
            content: &message.text,
        }));
        messages.push(ChatMessage {
            role: "user",
            content: user,
        });
        let body = goodwin_json::to_vec(&ChatInput {
            model: &self.settings.chat_model,
            messages,
        })?;
        let reply = self.response(
            "/chat/completions",
            "application/json",
            &[&body],
            JSON_LIMIT,
        )?;
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }
        #[derive(Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }
        let answer = goodwin_json::from_slice::<Response>(&reply)?
            .choices
            .into_iter()
            .next()
            .context("Chat returned no answer")?
            .message
            .content;
        ensure!(!answer.trim().is_empty(), "Chat returned an empty answer");
        Ok(answer)
    }

    pub(crate) fn synthesize(&self, answer: &str) -> Result<Vec<u8>> {
        let mut pcm = Vec::new();
        self.stream_speech(answer, |bytes| {
            pcm.try_reserve(bytes.len())?;
            pcm.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(pcm)
    }

    pub(crate) fn stream_speech(
        &self,
        answer: &str,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct SpeechInput<'a> {
            model: &'a str,
            input: &'a str,
            voice: &'a str,
            response_format: &'static str,
        }
        let body = goodwin_json::to_vec(&SpeechInput {
            model: &self.settings.tts_model,
            input: answer,
            voice: &self.settings.voice,
            response_format: "pcm",
        })?;
        self.post(
            "/audio/speech",
            "application/json",
            &[&body],
            SPEECH_LIMIT,
            |content_type, bytes| {
                let media_type = content_type
                    .unwrap_or_default()
                    .split(';')
                    .next()
                    .unwrap_or_default();
                ensure!(
                    ["audio/pcm", "audio/x-pcm", "application/octet-stream"]
                        .iter()
                        .any(|value| media_type.eq_ignore_ascii_case(value)),
                    "Expected raw 24 kHz PCM from /audio/speech"
                );
                consume(bytes)
            },
        )
    }

    fn response(
        &self,
        path: &str,
        content_type: &str,
        body: &[&[u8]],
        limit: usize,
    ) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        self.post(path, content_type, body, limit, |_, bytes| {
            data.try_reserve(bytes.len())?;
            data.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(data)
    }

    fn post(
        &self,
        path: &str,
        content_type: &str,
        body: &[&[u8]],
        limit: usize,
        chunk: impl FnMut(Option<&str>, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let url = format!("{}{path}", self.settings.base_url);
        let length = body
            .iter()
            .map(|part| part.len())
            .sum::<usize>()
            .to_string();
        let mut headers = vec![("Content-Type", content_type), ("Content-Length", &length)];
        let authorization;
        if !self.settings.api_key.is_empty() {
            authorization = format!("Bearer {}", self.settings.api_key);
            headers.push(("Authorization", &authorization));
        }
        self.transport
            .post(&url, &headers, body, limit, chunk)
            .with_context(|| format!("Provider request {path}"))
    }
}
