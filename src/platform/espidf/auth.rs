use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, ensure};
use argon2::{
    PasswordHash,
    password_hash::phc::{Output, Salt},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use esp_idf_svc::sys;

use crate::panel_core::{AccessState, SESSION_DURATION, session_cookie, valid_password_length};

const PASSWORD_ALGORITHM: &str = "pbkdf2-sha512";
const PASSWORD_ITERATIONS: u32 = 220_000;

pub(super) struct Auth {
    password: Option<String>,
    sessions: VecDeque<(String, Instant)>,
    next_login: Option<Instant>,
}

impl Auth {
    pub fn new(password: Option<String>) -> Self {
        Self {
            password,
            sessions: VecDeque::new(),
            next_login: None,
        }
    }

    pub fn access(&self, token: Option<&str>) -> AccessState {
        AccessState {
            protected: true,
            authorized: token.is_some_and(|token| {
                self.sessions
                    .iter()
                    .any(|(id, expiry)| id == token && Instant::now() < *expiry)
            }),
            setup_required: self.password.is_none(),
        }
    }

    pub fn hash(password: &str) -> Result<String> {
        let bytes = random::<16>()?;
        let salt = Salt::new(&bytes).map_err(|error| anyhow!("{error}"))?;
        let digest = password_digest(password, &bytes, PASSWORD_ITERATIONS)?;
        Ok(format!(
            "${PASSWORD_ALGORITHM}$i={PASSWORD_ITERATIONS}${salt}${digest}"
        ))
    }

    pub fn set_password(&mut self, hash: String) {
        self.password = Some(hash);
    }

    pub fn login(&mut self, password: &str) -> Result<(), &'static str> {
        let hash = self.password.as_ref().ok_or("setup_required")?;
        let now = Instant::now();
        if self.next_login.is_some_and(|next| now < next) {
            return Err("login_retry");
        }
        self.next_login = Some(now + Duration::from_secs(2));
        if !valid_password_length(password) {
            return Err("wrong_password");
        }
        let parsed = PasswordHash::new(hash).map_err(|error| {
            log::error!("Read saved password hash: {error}");
            "internal"
        })?;
        if parsed.algorithm.as_str() != PASSWORD_ALGORITHM {
            log::error!("Unsupported panel password hash algorithm");
            return Err("internal");
        }
        let verified = verify_digest(password, &parsed).map_err(|error| {
            log::error!("Verify panel password: {error}");
            "internal"
        })?;
        if verified {
            Ok(())
        } else {
            Err("wrong_password")
        }
    }

    pub fn create_session(&mut self) -> Result<String> {
        let token = URL_SAFE_NO_PAD.encode(random::<32>()?);
        let now = Instant::now();
        self.sessions.retain(|(_, expiry)| *expiry > now);
        if self.sessions.len() == 8 {
            self.sessions.pop_front();
        }
        self.sessions
            .push_back((token.clone(), now + SESSION_DURATION));
        Ok(session_cookie(&token, SESSION_DURATION.as_secs()))
    }

    pub fn logout(&mut self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.retain(|(id, _)| id != token);
        }
    }
}

fn password_digest(password: &str, salt: &[u8], iterations: u32) -> Result<Output> {
    let mut digest = [0; 64];
    // SAFETY: All pointers refer to live slices and the output buffer holds exactly key_length bytes.
    let result = unsafe {
        sys::mbedtls_pkcs5_pbkdf2_hmac_ext(
            sys::mbedtls_md_type_t_MBEDTLS_MD_SHA512,
            password.as_ptr(),
            password.len(),
            salt.as_ptr(),
            salt.len(),
            iterations,
            digest.len() as u32,
            digest.as_mut_ptr(),
        )
    };
    ensure!(result == 0, "Derive panel password hash: {result:#x}");
    Output::new(&digest).map_err(|error| anyhow!("{error}"))
}

fn verify_digest(password: &str, hash: &PasswordHash) -> Result<bool> {
    let salt = hash
        .salt
        .as_ref()
        .ok_or_else(|| anyhow!("Missing password salt"))?;
    let rounds = hash
        .params
        .get_decimal("i")
        .ok_or_else(|| anyhow!("Missing password cost"))?;
    let expected = hash
        .hash
        .as_ref()
        .ok_or_else(|| anyhow!("Missing password digest"))?;
    ensure!(
        salt.len() >= 16 && rounds >= PASSWORD_ITERATIONS && rounds <= 1_000_000,
        "Invalid password hash parameters"
    );
    Ok(&password_digest(password, salt, rounds)? == expected)
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow!("Could not generate random bytes"))?;
    Ok(bytes)
}
