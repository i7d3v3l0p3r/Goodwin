use anyhow::Result;
use goodwin_openai_compatible::{Protocol, Upstream};
use goodwin_plugin::{
    Plugin,
    provider::{
        Capability, ChatRequest, Completion, ModelCatalog, ModelsRequest, SpeechAudio,
        SpeechRequest, Transcription, TranscriptionRequest,
    },
    run,
    settings::read_settings,
};
use serde_json::{Value, json};

struct OpenAiCompatible(Upstream);

impl Plugin for OpenAiCompatible {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "providers": [Capability::Transcription, Capability::Chat, Capability::Speech],
            "modelCatalog": true,
        })
    }

    fn transcribe(&mut self, request: TranscriptionRequest) -> Result<Transcription> {
        self.0.transcribe(&read_settings()?, request)
    }

    fn complete(&mut self, request: ChatRequest) -> Result<Completion> {
        self.0.complete(&read_settings()?, request)
    }

    fn synthesize(&mut self, request: SpeechRequest) -> Result<SpeechAudio> {
        self.0.synthesize(&read_settings()?, request)
    }

    fn supports_speech_streaming(&self) -> bool {
        true
    }

    fn synthesize_stream(
        &mut self,
        request: SpeechRequest,
        emit: &mut dyn FnMut(SpeechAudio) -> Result<()>,
    ) -> Result<()> {
        self.0.synthesize_stream(&read_settings()?, request, emit)
    }

    fn models(&mut self, request: ModelsRequest) -> Result<ModelCatalog> {
        Ok(self.0.models(&read_settings()?, request.capability))
    }
}

fn main() -> Result<()> {
    run(OpenAiCompatible(Upstream::new(Protocol::OpenAi)?))
}
