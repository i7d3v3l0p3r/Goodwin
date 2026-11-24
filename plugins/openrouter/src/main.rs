use anyhow::Result;
use goodwin_openai_compatible::{Config, Protocol, Upstream};
#[cfg(target_arch = "wasm32")]
use goodwin_plugin::services::guest::read_settings;
#[cfg(not(target_arch = "wasm32"))]
use goodwin_plugin::settings::read_settings;
use goodwin_plugin::{
    Plugin,
    provider::{
        Capability, ChatRequest, Completion, ModelCatalog, ModelsRequest, SpeechAudio,
        SpeechRequest, Transcription, TranscriptionRequest,
    },
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
        let config = Config::openrouter();
        Self {
            key: config.key,
            transcription_model: config.transcription_model,
            chat_model: config.chat_model,
            speech_model: config.speech_model,
            voice: config.voice,
        }
    }
}

fn config() -> Result<Config> {
    let settings: Settings = read_settings()?;
    Ok(Config {
        base_url: Config::openrouter().base_url,
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

    fn supports_chat_streaming(&self) -> bool {
        true
    }

    fn complete_stream(
        &mut self,
        request: ChatRequest,
        emit: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        self.0.complete_stream(&config()?, request, emit)
    }

    fn synthesize(&mut self, request: SpeechRequest) -> Result<SpeechAudio> {
        self.0.synthesize(&config()?, request)
    }

    fn models(&mut self, request: ModelsRequest) -> Result<ModelCatalog> {
        Ok(self.0.models(&config()?, request.capability))
    }
}

fn open() -> Result<OpenRouter> {
    Ok(OpenRouter(Upstream::new(Protocol::OpenRouter)?))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    goodwin_plugin::run(open()?)
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
goodwin_plugin::wasm_plugin!(OpenRouter, open);
