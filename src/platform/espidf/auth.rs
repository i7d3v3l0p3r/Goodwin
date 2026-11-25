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

    pub fn set_password(&mut self, hash: String) {
        self.password = Some(hash);
    }

    pub fn login(&mut self, password: &str) -> Result<(), &'static str> {
        let hash = self.password.as_ref().ok_or("setup_required")?;
        if !password::uses_current_algorithm(hash) {
            return Err("password_reprovision_required");
        }
        let now = Instant::now();
        if self.next_login.is_some_and(|next| now < next) {
            return Err("login_retry");
        }
        self.next_login = Some(now + Duration::from_secs(2));
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
