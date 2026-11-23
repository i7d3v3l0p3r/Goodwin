use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::provider::{
    Capability, ChatRequest, Completion, MAX_RECORDING_BYTES, MAX_SPEECH_BYTES,
    MAX_SPEECH_CHUNK_BYTES, ModelSelection, SpeechAudio, SpeechRequest, Transcription,
    TranscriptionRequest,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    app::Failure,
    assistant::StageSettings,
    plugins::{Plugins, Process, SpeechReplies},
    wav::PcmAudio,
};

mod chat;
use chat::CompletionMessage;
pub(crate) use chat::ToolConversation;

pub(crate) struct Provider {
    process: Process,
    selection: ModelSelection,
}

pub(crate) struct SpeechStream {
    replies: SpeechReplies,
    format: Option<(u32, u16)>,
    received: usize,
}

impl SpeechStream {
    pub(crate) async fn next(&mut self) -> Result<Option<PcmAudio>> {
        let Some(reply) = self.replies.next().await? else {
            ensure!(self.received != 0, "Empty PCM stream");
            return Ok(None);
        };
        let audio = decode_speech(reply, MAX_SPEECH_CHUNK_BYTES)?;
        let format = (audio.sample_rate, audio.channels);
        ensure!(
            *self.format.get_or_insert(format) == format,
            "PCM format changed"
        );
        self.received += audio.data.len();
        ensure!(
            self.received <= MAX_SPEECH_BYTES,
            "PCM stream exceeds size limit"
        );
        Ok(Some(audio))
    }
}

impl Plugins {
    pub(crate) fn resolve_provider(
        &self,
        settings: &StageSettings,
        capability: Capability,
    ) -> Result<Provider, Failure> {
        let (process, model_catalog) = self
            .provider(&settings.provider, capability)
            .ok_or(Failure::ProviderUnavailable)?;
        if !model_catalog && !settings.selection.is_empty() {
            return Err(Failure::ProviderUnavailable);
        }
        Ok(Provider {
            process,
            selection: settings.selection.clone(),
        })
    }
}

impl Provider {
    #[cfg(test)]
    pub(crate) fn mock(process: Process) -> Self {
        Self {
            process,
            selection: ModelSelection::default(),
        }
    }

    pub(crate) fn failure(error: anyhow::Error) -> Failure {
        eprintln!("Provider plugin request: {error:#}");
        Failure::ProviderFailed
    }

    pub(crate) async fn transcribe(&self, wav: &[u8]) -> Result<String> {
        ensure!(wav.len() <= MAX_RECORDING_BYTES, "Recording is too large");
        let result: Transcription = self
            .call(
                Capability::Transcription,
                TranscriptionRequest {
                    wav_base64: STANDARD.encode(wav),
                    language: "ru".into(),
                    selection: self.selection.clone(),
                },
            )
            .await?;
        Ok(result.text.trim().to_owned())
    }

    pub(crate) async fn reply<'a>(
        &self,
        text: &str,
        instructions: &str,
        history: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<String> {
        let mut messages = vec![json!({"role": "system", "content": instructions})];
        messages.extend(
            history
                .into_iter()
                .map(|(role, content)| json!({"role": role, "content": content})),
        );
        messages.push(json!({"role": "user", "content": text}));
        self.complete(ChatRequest {
            messages,
            tools: Vec::new(),
            tool_choice: None,
            max_completion_tokens: 256,
            selection: ModelSelection::default(),
        })
        .await?
        .text()
    }

    pub(crate) async fn complete(&self, mut request: ChatRequest) -> Result<CompletionMessage> {
        request.selection = self.selection.clone();
        let result: Completion = self.call(Capability::Chat, request).await?;
        ensure!(result.message.is_object(), "Invalid completion message");
        Ok(CompletionMessage(result.message))
    }

    pub(crate) async fn synthesize(&self, text: &str) -> Result<PcmAudio> {
        let audio: SpeechAudio = self
            .call(Capability::Speech, self.speech_request(text))
            .await?;
        decode_speech(audio, MAX_SPEECH_BYTES)
    }

    pub(crate) fn stream_speech(&self, text: &str) -> Result<Option<SpeechStream>> {
        Ok(self
            .process
            .stream_speech(self.speech_request(text))?
            .map(|replies| SpeechStream {
                replies,
                format: None,
                received: 0,
            }))
    }

    fn speech_request(&self, text: &str) -> SpeechRequest {
        SpeechRequest {
            text: text.into(),
            selection: self.selection.clone(),
        }
    }

    async fn call<T: DeserializeOwned>(
        &self,
        capability: Capability,
        request: impl Serialize,
    ) -> Result<T> {
        let response = self
            .process
            .provider(capability, serde_json::to_value(request)?)
            .await?;
        serde_json::from_value(response).context("Invalid provider response")
    }
}

fn decode_speech(audio: SpeechAudio, limit: usize) -> Result<PcmAudio> {
    let data = STANDARD
        .decode(audio.pcm_base64)
        .context("Decode plugin PCM")?;
    ensure!(
        (8_000..=96_000).contains(&audio.sample_rate)
            && (1..=2).contains(&audio.channels)
            && !data.is_empty()
            && data.len() <= limit
            && data.len().is_multiple_of(2 * usize::from(audio.channels)),
        "Invalid plugin PCM"
    );
    Ok(PcmAudio {
        data,
        sample_rate: audio.sample_rate,
        channels: audio.channels,
    })
}
