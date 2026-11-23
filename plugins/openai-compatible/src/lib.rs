mod upstream;

use serde::Deserialize;

pub use upstream::{Protocol, Upstream};

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_url: String,
    pub transcription_model: String,
    pub chat_model: String,
    pub speech_model: String,
    pub voice: String,
    pub key: String,
}
