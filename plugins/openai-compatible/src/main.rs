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
#[cfg(target_arch = "wasm32")]
use serde::Deserialize;
use serde_json::{Value, json};

struct OpenAiCompatible(Upstream);

#[cfg(target_arch = "wasm32")]
#[derive(Deserialize)]
struct Settings {
    #[serde(default)]
    protocol: Protocol,
    #[serde(flatten)]
    config: Config,
}

impl OpenAiCompatible {
    #[cfg(not(target_arch = "wasm32"))]
    fn with_settings<T>(&self, call: impl FnOnce(&Upstream, &Config) -> Result<T>) -> Result<T> {
        call(&self.0, &read_settings()?)
    }

    #[cfg(target_arch = "wasm32")]
    fn with_settings<T>(&self, call: impl FnOnce(&Upstream, &Config) -> Result<T>) -> Result<T> {
        let Settings { protocol, config } = read_settings()?;
        let config = if protocol == Protocol::OpenRouter {
            Config {
                key: config.key,
                ..Config::openrouter()
            }
        } else {
            config
        };
        call(&self.0.with_protocol(protocol), &config)
    }
}

impl Plugin for OpenAiCompatible {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "providers": [Capability::Transcription, Capability::Chat, Capability::Speech],
            "modelCatalog": true,
        })
    }

    fn transcribe(&mut self, request: TranscriptionRequest) -> Result<Transcription> {
        self.with_settings(|upstream, config| upstream.transcribe(config, request))
    }

    fn complete(&mut self, request: ChatRequest) -> Result<Completion> {
        self.with_settings(|upstream, config| upstream.complete(config, request))
    }

    fn supports_chat_streaming(&self) -> bool {
        true
    }

    fn complete_stream(
        &mut self,
        request: ChatRequest,
        emit: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        self.with_settings(|upstream, config| upstream.complete_stream(config, request, emit))
    }

    fn synthesize(&mut self, request: SpeechRequest) -> Result<SpeechAudio> {
        self.with_settings(|upstream, config| upstream.synthesize(config, request))
    }

    fn supports_speech_streaming(&self) -> bool {
        true
    }

    fn synthesize_stream(
        &mut self,
        request: SpeechRequest,
        emit: &mut dyn FnMut(SpeechAudio) -> Result<()>,
    ) -> Result<()> {
        self.with_settings(|upstream, config| upstream.synthesize_stream(config, request, emit))
    }

    fn models(&mut self, request: ModelsRequest) -> Result<ModelCatalog> {
        self.with_settings(|upstream, config| Ok(upstream.models(config, request.capability)))
    }
}

fn open() -> Result<OpenAiCompatible> {
    Ok(OpenAiCompatible(Upstream::new(Protocol::OpenAi)?))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    goodwin_plugin::run(open()?)
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
goodwin_plugin::wasm_plugin!(OpenAiCompatible, open);
