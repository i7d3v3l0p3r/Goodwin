use anyhow::Result;
use goodwin_openai_compatible::{Config, Protocol, Upstream};
use goodwin_plugin::{
    Plugin,
    provider::{
        Capability, ChatRequest, Completion, ModelCatalog, ModelsRequest, SpeechAudio,
        SpeechRequest, Transcription, TranscriptionRequest,
    },
    run,
    settings::read_settings,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(default)]
struct Settings {
    key: String,
    transcription_model: String,
    chat_model: String,
    speech_model: String,
    voice: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            key: String::new(),
            transcription_model: "openai/whisper-large-v3".into(),
            chat_model: "openai/gpt-4.1-mini".into(),
            speech_model: "microsoft/mai-voice-2.1-flash".into(),
            voice: "ru-RU-Lev:MAI-Voice-2.1-Flash".into(),
        }
    }
}

fn config() -> Result<Config> {
    let settings: Settings = read_settings()?;
    Ok(Config {
        base_url: "https://openrouter.ai/api/v1".into(),
        key: settings.key,
        transcription_model: settings.transcription_model,
        chat_model: settings.chat_model,
        speech_model: settings.speech_model,
        voice: settings.voice,
    })
}

struct OpenRouter(Upstream);

impl Plugin for OpenRouter {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "providers": [Capability::Transcription, Capability::Chat, Capability::Speech],
            "modelCatalog": true,
        })
    }

    fn transcribe(&mut self, request: TranscriptionRequest) -> Result<Transcription> {
        self.0.transcribe(&config()?, request)
    }

    fn complete(&mut self, request: ChatRequest) -> Result<Completion> {
        self.0.complete(&config()?, request)
    }

    fn synthesize(&mut self, request: SpeechRequest) -> Result<SpeechAudio> {
        self.0.synthesize(&config()?, request)
    }

    fn models(&mut self, request: ModelsRequest) -> Result<ModelCatalog> {
        Ok(self.0.models(&config()?, request.capability))
    }
}

fn main() -> Result<()> {
    run(OpenRouter(Upstream::new(Protocol::OpenRouter)?))
}
