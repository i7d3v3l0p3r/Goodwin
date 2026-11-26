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

/// The raw, interleaved signed 16-bit little-endian PCM used by Goodwin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmFormat {
    pub const DEFAULT_RATE: u32 = 24_000;

    pub fn from_content_type(content_type: &str) -> Option<Self> {
        let mut parts = content_type.split(';').map(str::trim);
        let media_type = parts.next()?;
        if !["audio/pcm", "audio/x-pcm", "application/octet-stream"]
            .iter()
            .any(|supported| media_type.eq_ignore_ascii_case(supported))
        {
            return None;
        }
        let (mut rate, mut channels) = (None, None);
        for part in parts.filter(|part| !part.is_empty()) {
            let (name, value) = part.split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            let value = if let Some(quoted) = value.strip_prefix('"') {
                quoted.strip_suffix('"')?
            } else {
                value
            };
            if ["rate", "sample-rate", "sample_rate"]
                .iter()
                .any(|item| name.eq_ignore_ascii_case(item))
            {
                if rate.replace(value.parse::<u32>().ok()?).is_some() {
                    return None;
                }
            } else if name.eq_ignore_ascii_case("channels") {
                if channels.replace(value.parse::<u16>().ok()?).is_some() {
                    return None;
                }
            } else if ["bits", "bits-per-sample", "bit-depth"]
                .iter()
                .any(|item| name.eq_ignore_ascii_case(item))
            {
                if value != "16" {
                    return None;
                }
            } else if ["format", "encoding", "codec", "sample-format"]
                .iter()
                .any(|item| name.eq_ignore_ascii_case(item))
            {
                if !["pcm", "s16le", "pcm_s16le"]
                    .iter()
                    .any(|item| value.eq_ignore_ascii_case(item))
                {
                    return None;
                }
            } else if name.eq_ignore_ascii_case("endian")
                && !["le", "little", "little-endian"]
                    .iter()
                    .any(|item| value.eq_ignore_ascii_case(item))
            {
                return None;
            }
        }
        let sample_rate = rate.unwrap_or(Self::DEFAULT_RATE);
        let channels = channels.unwrap_or(1);
        if !(8_000..=96_000).contains(&sample_rate) || !(1..=2).contains(&channels) {
            return None;
        }
        Some(Self {
            sample_rate,
            channels,
        })
    }

    pub fn frame_bytes(self) -> usize {
        2 * usize::from(self.channels)
    }
}

#[cfg(test)]
mod pcm_tests {
    use super::PcmFormat;

    #[test]
    fn pcm_format_accepts_documented_defaults_and_explicit_compatible_metadata() {
        let default = PcmFormat::from_content_type("application/octet-stream").unwrap();
        assert_eq!(
            (default.sample_rate, default.channels, default.frame_bytes()),
            (24_000, 1, 2)
        );
        let stereo = PcmFormat::from_content_type(
            "Audio/PCM; rate=\"16000\"; channels=2; format=pcm_s16le; bits=16; endian=little",
        )
        .unwrap();
        assert_eq!(
            (stereo.sample_rate, stereo.channels, stereo.frame_bytes()),
            (16_000, 2, 4)
        );
    }

    #[test]
    fn pcm_format_rejects_conflicting_or_incompatible_metadata() {
        for value in [
            "audio/wav",
            "audio/L16;rate=24000",
            "audio/pcm;rate=0",
            "audio/pcm;rate=16000;RATE=24000",
            "audio/pcm;rate=16000;sample-rate=24000",
            "audio/pcm;channels=3",
            "audio/pcm;channels=1;channels=2",
            "audio/pcm;channels=\"1",
            "audio/pcm;bits=24",
            "audio/pcm;encoding=f32le",
            "audio/pcm;endian=big",
        ] {
            assert!(PcmFormat::from_content_type(value).is_none(), "{value}");
        }
    }
}
