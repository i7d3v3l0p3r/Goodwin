use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result, anyhow, ensure};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use serde::{Deserialize, Serialize};

use crate::openrouter::Models;

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
    pub models: Models,
    pub sensitivity: f32,
    pub listening: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            models: Models::default(),
            sensitivity: 0.5,
            listening: true,
        }
    }
}

impl Settings {
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            (0.0..=1.0).contains(&self.sensitivity),
            "Invalid sensitivity"
        );
        for field in [
            &mut self.models.transcription,
            &mut self.models.chat,
            &mut self.models.speech,
            &mut self.models.voice,
        ] {
            *field = field.trim().to_owned();
            ensure!(
                !field.is_empty() && field.len() <= 160 && !field.chars().any(char::is_control),
                "Invalid model or voice"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub access: Access,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub key: String,
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let file = fs::File::open(Self::path(dir))
            .context("Open config.json (run goodwin setup first)")?;
        let mut config: Self = serde_json::from_reader(file).context("Read config.json")?;
        config.settings.validate()?;
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

    pub fn key(&self) -> String {
        if self.key.is_empty() {
            std::env::var("OPENROUTER_API_KEY")
                .unwrap_or_default()
                .trim()
                .to_owned()
        } else {
            self.key.clone()
        }
    }
}

pub(crate) struct ConfigStore {
    dir: PathBuf,
    current: Mutex<Config>,
    writer: Mutex<()>,
}

impl ConfigStore {
    pub(crate) fn new(dir: &Path, config: Config) -> Self {
        Self {
            dir: dir.to_owned(),
            current: Mutex::new(config),
            writer: Mutex::new(()),
        }
    }

    fn current(&self) -> MutexGuard<'_, Config> {
        self.current.lock().expect("configuration mutex")
    }

    pub(crate) fn request_settings(&self) -> (Settings, String) {
        let config = self.current();
        (config.settings.clone(), config.key())
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
        Ok(Some(hash))
    }

    pub(crate) fn wake_settings(&self) -> (bool, f32) {
        let config = self.current();
        (config.settings.listening, config.settings.sensitivity)
    }

    pub(crate) fn update_settings(&self, settings: Settings, key: Option<String>) -> Result<()> {
        // Serialize the write so an update without a key uses the last committed key.
        let _writer = self.writer.lock().expect("configuration writer mutex");
        let mut draft = self.current().clone();
        draft.settings = settings;
        if let Some(key) = key {
            draft.key = key;
        }
        draft.save(&self.dir, false)?;
        *self.current() = draft;
        Ok(())
    }
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow!("Could not generate random bytes"))?;
    Ok(bytes)
}

pub(crate) fn valid_password_length(password: &str) -> bool {
    (8..=256).contains(&password.len())
}

pub fn hash_password(password: &str) -> Result<String> {
    ensure!(
        valid_password_length(password),
        "Use a password between 8 and 256 bytes"
    );
    let salt =
        SaltString::encode_b64(&random_bytes::<16>()?).map_err(|error| anyhow!("{error}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow!("{error}"))
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    password.len() <= 256
        && PasswordHash::new(hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
}
