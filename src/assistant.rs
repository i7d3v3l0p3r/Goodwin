use goodwin_error::{Result, ensure};
use goodwin_plugin::provider::{Capability, ModelSelection};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub(crate) mod chat;
pub(crate) mod speech;

#[cfg(not(target_os = "espidf"))]
use crate::{app::Failure, plugins::Plugins, providers::Provider};

pub(crate) const DEFAULT_PROVIDER: &str = "openai-compatible:openrouter";

pub(crate) fn decode_settings<T: DeserializeOwned>(input: &[u8]) -> Result<T> {
    match goodwin_json::from_slice(input) {
        Ok(value) => Ok(value),
        Err(error) if error.has_generic_message() => match serde_json::from_slice::<T>(input) {
            Err(detail) => Err(detail.into()),
            Ok(_) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Pipeline,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssistantSettings {
    pub engine: Engine,
    pub pipeline: PipelineSettings,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineSettings {
    pub transcription: StageSettings,
    pub chat: StageSettings,
    pub speech: StageSettings,
    pub speech_streaming: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StageSettings {
    pub provider: String,
    pub selection: ModelSelection,
}

impl Default for StageSettings {
    fn default() -> Self {
        Self {
            provider: DEFAULT_PROVIDER.into(),
            selection: ModelSelection::default(),
        }
    }
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            transcription: StageSettings::default(),
            chat: StageSettings::default(),
            speech: StageSettings::default(),
            speech_streaming: true,
        }
    }
}

impl PipelineSettings {
    pub fn get(&self, capability: Capability) -> &StageSettings {
        match capability {
            Capability::Transcription => &self.transcription,
            Capability::Chat => &self.chat,
            Capability::Speech => &self.speech,
        }
    }
}

impl AssistantSettings {
    #[cfg(not(target_os = "espidf"))]
    pub fn resolve(&self, plugins: &Plugins, capability: Capability) -> Result<Provider, Failure> {
        match self.engine {
            Engine::Pipeline => plugins.resolve_provider(self.pipeline.get(capability), capability),
        }
    }

    pub fn validate(&self) -> Result<()> {
        for stage in [
            &self.pipeline.transcription,
            &self.pipeline.chat,
            &self.pipeline.speech,
        ] {
            ensure!(
                crate::plugins::valid_provider_id(&stage.provider),
                "Invalid provider ID"
            );
            stage.selection.validate()?;
        }
        Ok(())
    }
}
