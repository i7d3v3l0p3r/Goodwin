#[cfg(all(target_arch = "wasm32", not(feature = "wasm-guest")))]
use alloc::string::String;
use serde::{Deserialize, Serialize};

pub const EVENT_METHOD: &str = "wake.event";

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub wake_word: String,
    pub kind: String,
    pub sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub stream: u64,
    pub index: u64,
    pub model: Model,
    pub ended_at_ms: u64,
    pub sample_rate: u32,
    pub gain: f32,
    pub threshold: f32,
    pub score: f32,
    /// Raw mono signed 16-bit little-endian PCM encoded as standard base64.
    pub pcm: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum Event {
    Frame(Frame),
    End { stream: u64 },
}

#[derive(Deserialize, Serialize)]
pub struct Observation<T = Event> {
    pub subscription: u64,
    pub event: T,
}
