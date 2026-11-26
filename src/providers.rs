use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Result, ensure};
use goodwin_plugin::provider::{
    Capability, ChatMessage, ChatRequest, MAX_RECORDING_BYTES, MAX_SPEECH_BYTES,
    MAX_SPEECH_CHUNK_BYTES, ModelSelection, SpeechAudio, SpeechRequest, Transcription,
    TranscriptionRequest,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    app::Failure,
    assistant::{StageSettings, chat, speech},
    panel_core::Message,
    plugins::{Plugins, Process, SpeechReplies},
    wav::PcmAudio,
};

pub(crate) struct Provider {
    process: Process,
    selection: ModelSelection,
    _package: Option<std::sync::Arc<()>>,
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
        let format = (reply.sample_rate, reply.channels);
        ensure!(
            *self.format.get_or_insert(format) == format,
            "PCM format changed"
        );
        let data = speech::decode_pcm(&reply, MAX_SPEECH_CHUNK_BYTES)?;
        self.received += data.len();
        ensure!(
            self.received <= MAX_SPEECH_BYTES,
            "PCM stream exceeds size limit"
        );
        Ok(Some(PcmAudio {
            sample_rate: reply.sample_rate,
            channels: reply.channels,
            data,
        }))
    }
}

impl Plugins {
    pub(crate) fn resolve_provider(
        &self,
        settings: &StageSettings,
        capability: Capability,
    ) -> Result<Provider, Failure> {
        let id = &settings.provider;
        let Some((process, model_catalog)) = self.provider(id, capability) else {
            log::warn!("Provider unavailable id={id} capability={capability:?}");
            return Err(Failure::ProviderUnavailable);
        };
        Provider::from_process(settings, capability, process, model_catalog)
    }
}

impl Provider {
    pub(crate) fn from_process(
        settings: &StageSettings,
        capability: Capability,
        process: Process,
        model_catalog: bool,
    ) -> Result<Self, Failure> {
        let id = &settings.provider;
        if !model_catalog && !settings.selection.is_empty() {
            log::warn!(
                "Provider does not accept model selection id={id} capability={capability:?}"
            );
            return Err(Failure::ProviderUnavailable);
        }
        Ok(Self {
            process,
            selection: settings.selection.clone(),
            _package: None,
        })
    }

    pub(crate) fn pin_package(mut self, package: std::sync::Arc<()>) -> Self {
        self._package = Some(package);
        self
    }
    #[cfg(test)]
    pub(crate) fn mock(process: Process) -> Self {
        Self {
            process,
            selection: ModelSelection::default(),
            _package: None,
        }
    }

    pub(crate) fn failure(error: goodwin_error::Error) -> Failure {
        log::warn!("Provider request failed: {error}");
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

    pub(crate) async fn reply(
        &self,
        text: &str,
        instructions: &str,
        history: Vec<Message>,
        on_chunk: Option<&mut (dyn FnMut(&str) + Send)>,
    ) -> Result<String> {
        let request = ChatRequest {
            selection: self.selection.clone(),
            ..chat::request(instructions, text, history)
        };
        if let Some(on_chunk) = on_chunk
            && let Some(mut stream) = self.process.stream_chat(&request)?
        {
            let mut answer = String::new();
            while let Some(chunk) = stream.next().await? {
                chat::append_chunk(&mut answer, &chunk)?;
                on_chunk(&chunk);
            }
            return chat::answer_text(answer);
        }
        let message = self.complete_request(&request).await?;
        chat::answer_text(message.content.unwrap_or_default())
    }

    async fn complete_request(&self, request: &ChatRequest) -> Result<ChatMessage> {
        chat::assistant_message(self.call(Capability::Chat, request).await?)
    }

    pub(crate) async fn synthesize_wav(&self, text: &str) -> Result<Vec<u8>> {
        let audio: SpeechAudio = self
            .call(Capability::Speech, self.speech_request(text))
            .await?;
        speech::decode_wav(audio)
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
        self.process.provider(capability, request).await
    }
}

impl crate::voice_core::tools::ToolChat for Provider {
    async fn complete(&self, request: &mut ChatRequest) -> Result<ChatMessage> {
        request.selection.clone_from(&self.selection);
        self.complete_request(request).await
    }
}
