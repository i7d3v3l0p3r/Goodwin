#[cfg(all(target_arch = "wasm32", not(feature = "wasm-guest")))]
use alloc::{string::String, vec::Vec};
use core::fmt;
use serde::{Deserialize, Serialize};
mod chat;
mod parameters;

pub use chat::{
    ChatCompletion, ChatMessage, ChatPayload, ChatRole, FunctionCall, ReasoningDetail, ToolCall,
    ToolDefinition, ToolKind, ToolParameterKind, ToolProperty, ToolSchema, ToolSchemaKind,
};
pub use parameters::ModelParameters;

pub const MAX_RECORDING_BYTES: usize = 4_000_000;
pub const MAX_SPEECH_BYTES: usize = 8_000_000;
pub const MODELS_METHOD: &str = "provider.models";
pub const CHAT_STREAM_METHOD: &str = "provider.complete_stream";
pub const CHAT_CHUNK_NOTIFICATION: &str = "provider.chat_chunk";
pub const MAX_CHAT_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_CHAT_BYTES: usize = 256_000;
pub const SPEECH_STREAM_METHOD: &str = "provider.synthesize_stream";
pub const SPEECH_CHUNK_NOTIFICATION: &str = "provider.speech_chunk";
pub const MAX_SPEECH_CHUNK_BYTES: usize = 16 * 1024;

/// An upstream HTTP status that can be reported to the host without the response body.
#[derive(Debug)]
pub struct ProviderHttpError(pub u16);

impl core::fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "Provider returned HTTP {}", self.0)
    }
}

impl core::error::Error for ProviderHttpError {}

#[derive(Debug)]
pub enum ModelSelectionError {
    InvalidId,
    ParametersTooLarge,
    InvalidParameters,
}

impl fmt::Display for ModelSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidId => "Invalid model ID",
            Self::ParametersTooLarge => "Model parameters are too large",
            Self::InvalidParameters => "Invalid model parameters",
        };
        formatter.write_str(message)
    }
}

impl core::error::Error for ModelSelectionError {}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Transcription,
    Chat,
    Speech,
}

impl Capability {
    pub const fn method(self) -> &'static str {
        match self {
            Self::Transcription => "provider.transcribe",
            Self::Chat => "provider.complete",
            Self::Speech => "provider.synthesize",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "ModelParameters::is_empty")]
    pub parameters: ModelParameters,
}

impl ModelSelection {
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.parameters.is_empty()
    }

    pub fn validate(&self) -> Result<(), ModelSelectionError> {
        if self.model.as_ref().is_some_and(|id| {
            id.is_empty() || id.trim() != id || id.len() > 256 || id.chars().any(char::is_control)
        }) {
            return Err(ModelSelectionError::InvalidId);
        }
        self.parameters.validate()
    }
}

#[derive(Deserialize, Serialize)]
pub struct ModelsRequest {
    pub capability: Capability,
}

#[derive(Deserialize, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<Model>,
    pub default: Option<String>,
    pub available: bool,
}

#[derive(Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voices: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionRequest {
    pub wav_base64: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "ModelSelection::is_empty")]
    pub selection: ModelSelection,
}

#[derive(Deserialize, Serialize)]
pub struct Transcription {
    pub text: String,
}

#[derive(Deserialize, Serialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    pub max_completion_tokens: u32,
    #[serde(default, skip_serializing_if = "ModelSelection::is_empty")]
    pub selection: ModelSelection,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    None,
}

#[derive(Deserialize, Serialize)]
pub struct Completion {
    pub message: ChatMessage,
}

#[derive(Deserialize, Serialize)]
pub struct SpeechRequest {
    pub text: String,
    #[serde(default, skip_serializing_if = "ModelSelection::is_empty")]
    pub selection: ModelSelection,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechAudio {
    pub pcm_base64: String,
    pub sample_rate: u32,
    pub channels: u16,
}
