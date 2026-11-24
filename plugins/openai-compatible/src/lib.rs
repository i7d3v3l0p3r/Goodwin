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

impl Config {
    pub fn openrouter() -> Self {
        Self {
            base_url: "https://openrouter.ai/api/v1".into(),
            transcription_model: "openai/whisper-large-v3".into(),
            chat_model: "openai/gpt-4.1-mini".into(),
            speech_model: "microsoft/mai-voice-2.1-flash".into(),
            voice: "ru-RU-Lev:MAI-Voice-2.1-Flash".into(),
            key: String::new(),
        }
    }
}
