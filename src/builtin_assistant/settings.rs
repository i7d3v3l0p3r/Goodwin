use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct ProviderSettings {
    pub base_url: String,
    pub api_key: String,
    pub stt_model: String,
    pub chat_model: String,
    pub tts_model: String,
    pub voice: String,
    #[serde(default)]
    pub speech_streaming: bool,
}

#[derive(Deserialize)]
pub(crate) struct ProviderUpdate {
    base_url: String,
    api_key: Option<String>,
    stt_model: String,
    chat_model: String,
    tts_model: String,
    voice: String,
    speech_streaming: Option<bool>,
}

impl ProviderSettings {
    pub(crate) fn configured(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub(crate) fn https(&self) -> bool {
        self.base_url.starts_with("https://")
    }

    pub(crate) fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "base_url": self.base_url, "has_key": !self.api_key.is_empty(),
            "stt_model": self.stt_model, "chat_model": self.chat_model,
            "tts_model": self.tts_model, "voice": self.voice,
            "speech_streaming": self.speech_streaming,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (required, value) in [
            (true, &self.base_url),
            (false, &self.api_key),
            (true, &self.stt_model),
            (true, &self.chat_model),
            (true, &self.tts_model),
            (true, &self.voice),
        ] {
            ensure!(
                (!required || !value.is_empty())
                    && value.len() <= 256
                    && !value.chars().any(char::is_control),
                "Invalid provider settings"
            );
        }
        let fixture = cfg!(feature = "esp-emulator")
            && self.base_url == "http://192.168.4.1:8765/v1"
            && self.api_key.is_empty();
        ensure!(
            fixture
                || self
                    .base_url
                    .strip_prefix("https://")
                    .is_some_and(|address| {
                        !address.is_empty()
                            && !address.starts_with('/')
                            && !address.contains(['?', '#', '@', ' ', '\\'])
                    }),
            "The provider base URL must be HTTPS"
        );
        Ok(())
    }

    pub(crate) fn update(&self, input: ProviderUpdate) -> Result<Self> {
        let base_url = input.base_url.trim().trim_end_matches('/').to_owned();
        let same_provider = base_url == self.base_url;
        let next = Self {
            base_url,
            api_key: input.api_key.unwrap_or_else(|| {
                if same_provider {
                    self.api_key.clone()
                } else {
                    String::new()
                }
            }),
            stt_model: input.stt_model.trim().to_owned(),
            chat_model: input.chat_model.trim().to_owned(),
            tts_model: input.tts_model.trim().to_owned(),
            voice: input.voice.trim().to_owned(),
            speech_streaming: input.speech_streaming.unwrap_or(self.speech_streaming),
        };
        next.validate()?;
        Ok(next)
    }
}
