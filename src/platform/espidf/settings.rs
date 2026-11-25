use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
use goodwin_error::{Context, Result};

use crate::builtin_assistant::{AudioSettings, ProviderSettings};

use super::network::WifiCredentials;

pub(super) struct Store {
    nvs: EspDefaultNvs,
}

impl Store {
    pub(super) fn open(partition: EspDefaultNvsPartition) -> Result<Self> {
        Ok(Self {
            nvs: EspDefaultNvs::new(partition, "goodwin", true)?,
        })
    }

    fn read(&self, key: &str, bytes: &mut [u8]) -> Result<String> {
        Ok(self.nvs.get_str(key, bytes)?.unwrap_or_default().to_owned())
    }

    pub(super) fn wifi(&self) -> Result<Option<WifiCredentials>> {
        let saved = self.read("wifi", &mut [0; 512])?;
        if !saved.is_empty() {
            return goodwin_json::from_str::<WifiCredentials>(&saved)?
                .validate()
                .map(Some);
        }
        let mut bytes = [0; 257];
        let ssid = self.read("ssid", &mut bytes)?;
        if ssid.is_empty() {
            return Ok(None);
        }
        WifiCredentials {
            ssid,
            password: self.read("wifi_password", &mut bytes)?,
        }
        .validate()
        .map(Some)
    }

    pub(super) fn save_wifi(&mut self, settings: &WifiCredentials) -> Result<()> {
        self.nvs
            .set_str("wifi", &goodwin_json::to_string(settings)?)?;
        Ok(())
    }

    pub(super) fn provider(&self) -> Result<ProviderSettings> {
        let mut bytes = [0; 4096];
        let saved = self.read("provider", &mut bytes)?;
        let provider = if saved.is_empty() {
            let base_url = self.read("base_url", &mut bytes)?;
            if base_url.is_empty() {
                return Ok(ProviderSettings::default());
            }
            ProviderSettings {
                base_url: base_url.trim_end_matches('/').to_owned(),
                api_key: self.read("api_key", &mut bytes)?,
                stt_model: self.read("stt_model", &mut bytes)?,
                chat_model: self.read("chat_model", &mut bytes)?,
                tts_model: self.read("tts_model", &mut bytes)?,
                voice: self.read("voice", &mut bytes)?,
                speech_streaming: false,
            }
        } else {
            goodwin_json::from_str(&saved).context("Read saved provider")?
        };
        provider.validate()?;
        Ok(provider)
    }

    pub(super) fn save_provider(&mut self, settings: &ProviderSettings) -> Result<()> {
        let data = goodwin_json::to_string(settings)?;
        self.nvs.set_str("provider", &data)?;
        Ok(())
    }

    pub(super) fn system_prompt(&self) -> Result<String> {
        let Some(length) = self.nvs.blob_len("system_prompt")? else {
            return Ok(String::new());
        };
        goodwin_error::ensure!(length <= 12 * 1024, "System prompt is too large");
        let mut bytes = vec![0; length];
        let value = self
            .nvs
            .get_blob("system_prompt", &mut bytes)?
            .context("Read system prompt")?;
        crate::panel_core::normalize_system_prompt(std::str::from_utf8(value)?)
    }

    pub(super) fn save_system_prompt(&mut self, prompt: &str) -> Result<()> {
        if prompt.is_empty() {
            self.nvs.remove("system_prompt")?;
        } else {
            self.nvs.set_blob("system_prompt", prompt.as_bytes())?;
        }
        Ok(())
    }

    #[cfg(feature = "wake-tflite")]
    pub(super) fn wake_selection(&self) -> Result<Option<crate::wake_catalog::Selection>> {
        let Some(length) = self.nvs.blob_len("wake_model")? else {
            return Ok(None);
        };
        goodwin_error::ensure!(length <= 1024, "Saved wake model metadata is too large");
        let mut bytes = vec![0; length];
        let value = self
            .nvs
            .get_blob("wake_model", &mut bytes)?
            .context("Read wake model metadata")?;
        Ok(Some(goodwin_json::from_slice(value)?))
    }

    #[cfg(feature = "wake-tflite")]
    pub(super) fn save_wake_selection(
        &mut self,
        selection: &crate::wake_catalog::Selection,
    ) -> Result<()> {
        self.nvs
            .set_blob("wake_model", &goodwin_json::to_vec(selection)?)?;
        Ok(())
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn assistant(&self) -> Result<Option<crate::assistant::AssistantSettings>> {
        let Some(length) = self.nvs.blob_len("assistant")? else {
            return Ok(None);
        };
        goodwin_error::ensure!(length <= 8 * 1024, "Assistant settings are too large");
        let mut bytes = vec![0; length];
        let value = self
            .nvs
            .get_blob("assistant", &mut bytes)?
            .context("Read assistant settings")?;
        let settings: crate::assistant::AssistantSettings =
            crate::assistant::decode_settings(value)?;
        settings.validate()?;
        Ok(Some(settings))
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn save_assistant(
        &mut self,
        settings: &crate::assistant::AssistantSettings,
    ) -> Result<()> {
        self.nvs
            .set_blob("assistant", &goodwin_json::to_vec(settings)?)?;
        Ok(())
    }

    pub(super) fn audio(&self) -> Result<AudioSettings> {
        let saved = self.read("audio_settings", &mut [0; 512])?;
        if saved.is_empty() {
            return Ok(AudioSettings::default());
        }
        goodwin_json::from_str::<AudioSettings>(&saved)?.validate()
    }

    pub(super) fn save_audio(&mut self, settings: AudioSettings) -> Result<()> {
        self.nvs
            .set_str("audio_settings", &goodwin_json::to_string(&settings)?)?;
        Ok(())
    }

    pub(super) fn password(&self) -> Result<Option<String>> {
        let mut bytes = [0; 256];
        if let Some(hash) = self.nvs.get_str("panel_pbkdf2", &mut bytes)? {
            return Ok(Some(hash.to_owned()));
        }
        Ok(self
            .nvs
            .get_str("panel_password", &mut bytes)?
            .map(str::to_owned))
    }

    pub(super) fn save_password(&mut self, hash: &str) -> Result<()> {
        self.nvs.set_str("panel_pbkdf2", hash)?;
        Ok(())
    }
}
