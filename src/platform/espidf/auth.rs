use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use goodwin_error::Result;

use crate::{
    panel_core::{AccessState, SESSION_DURATION, random_bytes, session_cookie},
    password,
};

pub(super) struct Auth {
    mode: Mode,
    sessions: VecDeque<(String, Instant)>,
    next_login: Option<Instant>,
}

enum Mode {
    Pending,
    Open,
    Password(String),
}

pub(super) const OPEN_ACCESS: &str = "open";

impl Auth {
    pub fn new(password: Option<String>) -> Self {
        let mode = match password {
            None => Mode::Pending,
            Some(hash) if hash == OPEN_ACCESS => Mode::Open,
            Some(hash) => Mode::Password(hash),
        };
        Self {
            mode,
            sessions: VecDeque::new(),
            next_login: None,
        }
    }

    pub fn access(&self, token: Option<&str>) -> AccessState {
        let protected = !matches!(self.mode, Mode::Open);
        AccessState {
            protected,
            authorized: !protected
                || token.is_some_and(|token| {
                    self.sessions
                        .iter()
                        .any(|(id, expiry)| id == token && Instant::now() < *expiry)
                }),
            setup_required: matches!(self.mode, Mode::Pending),
        }
    }

    pub fn set_password(&mut self, hash: String) {
        self.mode = Mode::Password(hash);
        self.sessions.clear();
        self.next_login = None;
    }

    pub fn set_open(&mut self) {
        self.mode = Mode::Open;
        self.sessions.clear();
        self.next_login = None;
    }

    pub fn reserve_login(&mut self) -> Result<String, &'static str> {
        let Mode::Password(hash) = &self.mode else {
            return Err("setup_required");
        };
        if !password::supports_hash(hash) {
            return Err("password_reprovision_required");
        }
        let now = Instant::now();
        if self.next_login.is_some_and(|next| now < next) {
            return Err("login_retry");
        }
        self.next_login = Some(now + Duration::from_secs(2));
        Ok(hash.clone())
    }

    pub fn verify_login(hash: &str, password: &str) -> Result<(), &'static str> {
        let verified = password::verify(hash, password).map_err(|error| {
            log::error!("Verify panel password: {error:#}");
            "internal"
        })?;
        if verified {
            Ok(())
        } else {
            Err("wrong_password")
        }
    }

    pub fn uses_hash(&self, hash: &str) -> bool {
        matches!(&self.mode, Mode::Password(current) if current == hash)
    }

    pub fn create_session(&mut self) -> Result<String> {
        let token = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
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
