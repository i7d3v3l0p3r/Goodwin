use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result, anyhow, ensure};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::assistant::AssistantSettings;

pub(crate) const DEFAULT_MICROPHONE_GAIN: f32 = 4.0;

#[cfg(test)]
mod tests;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Access {
    Open,
    PendingPassword,
    Password { hash: String },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub assistant: AssistantSettings,
    pub sensitivity: f32,
    pub listening: bool,
    pub microphone_muted: bool,
    pub wake_gain: Option<f32>,
    pub transcription_gain: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            assistant: AssistantSettings::default(),
            sensitivity: 0.5,
            listening: true,
            microphone_muted: false,
            wake_gain: None,
            transcription_gain: None,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (0.0..=1.0).contains(&self.sensitivity),
            "Invalid sensitivity"
        );
        ensure!(
            [self.wake_gain, self.transcription_gain]
                .into_iter()
                .flatten()
                .all(valid_microphone_gain),
            "Microphone gain must be between 0.1 and 64"
        );
        self.assistant.validate()
    }
}

pub(crate) fn valid_microphone_gain(gain: f32) -> bool {
    (0.1..=64.0).contains(&gain)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub access: Access,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub system_prompt: String,
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    pub fn load(dir: &Path) -> Result<Self> {
        let file = fs::File::open(Self::path(dir))
            .context("Open config.json (run goodwin setup first)")?;
        let mut config: Self = serde_json::from_reader(file).context("Read config.json")?;
        config.settings.validate()?;
        config.system_prompt = normalize_system_prompt(&config.system_prompt)?;
        Ok(config)
    }

    pub fn save(&self, dir: &Path, initial: bool) -> Result<()> {
        fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        writeln!(file)?;
        file.as_file().sync_all()?;
        if initial {
            file.persist_noclobber(Self::path(dir))?;
        } else {
            file.persist(Self::path(dir))?;
        }
        Ok(())
    }
}

pub(crate) use crate::panel_core::normalize_system_prompt;

#[derive(Clone)]
pub(crate) struct RequestSettings {
    pub settings: Settings,
    pub system_prompt: String,
}

pub(crate) struct ConfigStore {
    dir: PathBuf,
    current: Mutex<Config>,
    writer: Mutex<()>,
    changes: watch::Sender<()>,
}

impl ConfigStore {
    pub(crate) fn new(dir: &Path, config: Config) -> Self {
        Self {
            dir: dir.to_owned(),
            current: Mutex::new(config),
            writer: Mutex::new(()),
            changes: watch::Sender::default(),
        }
    }

    pub(crate) fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    fn current(&self) -> MutexGuard<'_, Config> {
        self.current.lock().expect("configuration mutex")
    }

    pub(crate) fn request_settings(&self) -> RequestSettings {
        let config = self.current();
        RequestSettings {
            settings: config.settings.clone(),
            system_prompt: config.system_prompt.clone(),
        }
    }

    pub(crate) fn password_protected(&self) -> bool {
        matches!(self.current().access, Access::Password { .. })
    }

    pub(crate) fn access(&self) -> Access {
        self.current().access.clone()
    }

    pub(crate) fn set_initial_password(&self, password: &str) -> Result<Option<String>> {
        let _writer = self.writer.lock().expect("configuration writer mutex");
        let mut draft = self.current().clone();
        if !matches!(draft.access, Access::PendingPassword) {
            return Ok(None);
        }
        let hash = hash_password(password)?;
        draft.access = Access::Password { hash: hash.clone() };
        draft.save(&self.dir, false)?;
        *self.current() = draft;
        self.changes.send_replace(());
        Ok(Some(hash))
    }

    pub(crate) fn wake_settings(&self) -> (bool, f32, Option<f32>) {
        let config = self.current();
        (
            config.settings.listening,
            config.settings.sensitivity,
            config.settings.wake_gain,
        )
    }

    pub(crate) fn update_settings(&self, settings: Settings) -> Result<()> {
        self.update(|draft| {
            draft.settings = Settings {
                microphone_muted: draft.settings.microphone_muted,
                ..settings
            };
        })
    }

    pub(crate) fn update_microphone_muted(&self, muted: bool) -> Result<()> {
        self.update(|draft| draft.settings.microphone_muted = muted)
    }

    pub(crate) fn update_system_prompt(&self, prompt: String) -> Result<()> {
        self.update(|draft| draft.system_prompt = prompt)
    }

    fn update(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let _writer = self.writer.lock().expect("configuration writer mutex");
        let mut draft = self.current().clone();
        change(&mut draft);
        draft.save(&self.dir, false)?;
        *self.current() = draft;
        self.changes.send_replace(());
        Ok(())
    }
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow!("Could not generate random bytes"))?;
    Ok(bytes)
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
pub fn hash_password(password: &str) -> Result<String> {
    ensure!(
        crate::panel_core::valid_password_length(password),
        "Use a password between 8 and 256 bytes"
    );
    let salt =
        SaltString::encode_b64(&random_bytes::<16>()?).map_err(|error| anyhow!("{error}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow!("{error}"))
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
pub fn verify_password(hash: &str, password: &str) -> bool {
    password.len() <= 256
        && PasswordHash::new(hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
}
