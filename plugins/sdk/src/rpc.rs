//! JSON-RPC envelopes shared by the native and WASM plugin entry points.

extern crate alloc;

use alloc::string::String;
use core::{error::Error, fmt};
use serde::de::DeserializeOwned;

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-guest"))]
#[derive(serde::Serialize)]
#[serde(untagged)]
pub(crate) enum Payload {
    Empty(()),
    Json(serde_json::Value),
    Http(crate::HttpResponse),
    Models(crate::provider::ModelCatalog),
    Transcription(crate::provider::Transcription),
    Chat(crate::provider::Completion),
    Speech(crate::provider::SpeechAudio),
}

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-guest"))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub jsonrpc: &'static str,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Payload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_subscription: Option<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Serialize)]
pub(crate) struct Notification<T> {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: T,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatChunk<'a> {
    pub request_id: u64,
    pub text: &'a str,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeechChunk {
    pub request_id: u64,
    pub audio: crate::provider::SpeechAudio,
}
#[cfg(not(target_os = "espidf"))]
use serde_json::value::RawValue;

use crate::json_codec;

#[cfg(not(target_os = "espidf"))]
const BULK_FRAME: usize = 128 * 1024;

#[derive(Debug)]
#[doc(hidden)]
pub enum FrameError {
    MissingVersion,
    InvalidVersion,
    MissingId,
    MissingMethod,
    MissingParameters,
    TypedJson(json_codec::DecodeError),
    DynamicJson(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingVersion => "Missing RPC version",
            Self::InvalidVersion => "Unexpected RPC version",
            Self::MissingId => "Missing RPC request ID",
            Self::MissingMethod => "Missing RPC method",
            Self::MissingParameters => "Missing RPC parameters",
            Self::TypedJson(error) => return error.fmt(formatter),
            Self::DynamicJson(error) => return error.fmt(formatter),
        };
        formatter.write_str(message)
    }
}

impl Error for FrameError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TypedJson(error) => error.source(),
            Self::DynamicJson(error) => error.source(),
            _ => None,
        }
    }
}

impl From<json_codec::DecodeError> for FrameError {
    fn from(error: json_codec::DecodeError) -> Self {
        Self::TypedJson(error)
    }
}

impl From<serde_json::Error> for FrameError {
    fn from(error: serde_json::Error) -> Self {
        Self::DynamicJson(error)
    }
}

#[doc(hidden)]
pub struct Frame<'input> {
    pub id: u64,
    pub method: String,
    pub now_ms: Option<u64>,
    pub timer_clock: Option<crate::timer::ClockSample>,
    params: &'input [u8],
}

impl<'input> Frame<'input> {
    pub fn read(input: &'input [u8]) -> Result<Self, FrameError> {
        #[cfg(not(target_os = "espidf"))]
        if input.len() > BULK_FRAME {
            return Self::read_bulk(input);
        }
        let [version, id, method, params, now_ms, timer_clock] = json_codec::object_fields(
            input,
            ["jsonrpc", "id", "method", "params", "nowMs", "timerClock"],
        )?;
        let version: Option<String> = version.map(json_codec::from_slice).transpose()?;
        check_version(version.as_deref())?;
        Ok(Self {
            id: json_codec::from_slice(id.ok_or(FrameError::MissingId)?)?,
            method: json_codec::from_slice(method.ok_or(FrameError::MissingMethod)?)?,
            now_ms: now_ms.map(json_codec::from_slice).transpose()?,
            timer_clock: timer_clock.map(json_codec::from_slice).transpose()?,
            params: params.ok_or(FrameError::MissingParameters)?,
        })
    }

    #[cfg(not(target_os = "espidf"))]
    fn read_bulk(input: &'input [u8]) -> Result<Self, FrameError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Bulk<'a> {
            jsonrpc: Option<String>,
            id: u64,
            method: String,
            now_ms: Option<u64>,
            timer_clock: Option<crate::timer::ClockSample>,
            #[serde(borrow)]
            params: &'a RawValue,
        }

        // A single transcription request can carry several MiB of base64.
        // Borrow the payload and use the bulk scanner for these frames.
        let frame: Bulk<'_> = serde_json::from_slice(input)?;
        check_version(frame.jsonrpc.as_deref())?;
        Ok(Self {
            id: frame.id,
            method: frame.method,
            now_ms: frame.now_ms,
            timer_clock: frame.timer_clock,
            params: frame.params.get().as_bytes(),
        })
    }

    pub fn params<T: DeserializeOwned>(&self) -> Result<T, FrameError> {
        #[cfg(not(target_os = "espidf"))]
        if self.params.len() > BULK_FRAME {
            return Ok(serde_json::from_slice(self.params)?);
        }
        match json_codec::from_slice(self.params) {
            Ok(value) => Ok(value),
            // Third-party HTTP bodies, tool arguments and plugin state
            // can carry their own JSON schemas.
            Err(error) if error.requires_untyped_values() => {
                Ok(serde_json::from_slice(self.params)?)
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn check_version(version: Option<&str>) -> Result<(), FrameError> {
    match version {
        Some("2.0") => Ok(()),
        Some(_) => Err(FrameError::InvalidVersion),
        // WASM guests accepted frames without a version before the shared codec.
        None if cfg!(target_arch = "wasm32") => Ok(()),
        None => Err(FrameError::MissingVersion),
    }
}

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-guest"))]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Initialize {
    pub api_version: u32,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub speech_streaming: bool,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub chat_streaming: bool,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub wake_events: bool,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub timer_alarms: bool,
}
