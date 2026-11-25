use std::{
    collections::HashMap,
    fs, io,
    path::Path,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use blake2::{Blake2s256, Digest};
use tokio::sync::watch;

use crate::{
    config::{Access, random_bytes, verify_password},
    panel_core::{
        AccessState, CONTENT_SECURITY_POLICY, Login, SESSION_DURATION, SetupPassword, same_origin,
        session_cookie as cookie, session_token, valid_password_length,
    },
};

use super::{Panel, problem};

#[cfg(test)]
mod tests;

const SESSION_FILE: &str = "panel-sessions.json";

#[derive(Default)]
pub(super) struct Auth {
    sessions: HashMap<String, SystemTime>,
    next_login: Option<Instant>,
    pub(super) changes: watch::Sender<()>,
}

impl Auth {
    pub(super) fn load(dir: &Path) -> Self {
        let sessions = match fs::read(dir.join(SESSION_FILE)) {
            Ok(data) => serde_json::from_slice(&data).context("Read panel sessions"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(error) => Err(error).context("Open panel sessions"),
        };
        Self {
            sessions: sessions.unwrap_or_else(|error| {
                log::warn!("Could not restore panel sessions: {error}");
                HashMap::new()
            }),
            ..Self::default()
        }
    }

    fn save(&mut self, dir: &Path, sessions: HashMap<String, SystemTime>) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer(&mut file, &sessions)?;
        file.as_file().sync_all()?;
        file.persist(dir.join(SESSION_FILE))?;
        self.sessions = sessions;
        self.changes.send_replace(());
        Ok(())
    }
}

pub(super) async fn response_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    response
}

pub(super) async fn require_same_origin(request: axum::extract::Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/api/") {
        return next.run(request).await;
    }
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
        });
    let origin = headers
        .get(header::ORIGIN)
        .map(|value| value.to_str().unwrap_or_default());
    let site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if !same_origin(origin, host, site) {
        return problem(StatusCode::FORBIDDEN, "origin");
    }
    next.run(request).await
}

fn token(headers: &HeaderMap) -> Option<&str> {
    session_token(headers.get(header::COOKIE)?.to_str().ok()?)
}

async fn authorized(panel: &Panel, headers: &HeaderMap) -> bool {
    access_state(panel, headers).await.0.authorized
}

fn session_key(password_hash: &str, token: &str) -> String {
    // Binding the token to the configured hash revokes it when the panel password changes.
    URL_SAFE_NO_PAD.encode(
        Blake2s256::new()
            .chain_update(password_hash)
            .chain_update(token)
            .finalize(),
    )
}

async fn has_session(panel: &Panel, headers: &HeaderMap, password_hash: &str) -> bool {
    session_expiry(panel, headers, password_hash)
        .await
        .is_some()
}

async fn session_expiry(
    panel: &Panel,
    headers: &HeaderMap,
    password_hash: &str,
) -> Option<SystemTime> {
    let token = token(headers)?;
    let key = session_key(password_hash, token);
    panel
        .auth
        .lock()
        .await
        .sessions
        .get(&key)
        .copied()
        .filter(|expiry| SystemTime::now() < *expiry)
}

pub(super) async fn require_password_owner(
    State(panel): State<Panel>,
    request: Request,
    next: Next,
) -> Response {
    match password_rejection(&panel, request.headers(), "plugin_password_required").await {
        None => next.run(request).await,
        Some(response) => response,
    }
}

pub(super) async fn password_rejection(
    panel: &Panel,
    headers: &HeaderMap,
    required: &'static str,
) -> Option<Response> {
    let Access::Password { hash } = panel.app.configuration().access() else {
        return Some(problem(StatusCode::FORBIDDEN, required));
    };
    if !has_session(panel, headers, &hash).await {
        return Some(problem(StatusCode::UNAUTHORIZED, "login_required"));
    }
    None
}

pub(super) async fn require_owner(
    State(panel): State<Panel>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if authorized(&panel, request.headers()).await {
        next.run(request).await
    } else {
        problem(StatusCode::UNAUTHORIZED, "login_required")
    }
}

pub(super) async fn access_state(
    panel: &Panel,
    headers: &HeaderMap,
) -> (AccessState, Option<SystemTime>) {
    let (protected, setup_required, expires) = match panel.app.configuration().access() {
        Access::Open => (false, false, None),
        Access::PendingPassword => (true, true, None),
        Access::Password { hash } => (true, false, session_expiry(panel, headers, &hash).await),
    };
    let state = AccessState {
        protected,
        authorized: !protected || expires.is_some(),
        setup_required,
    };
    (state, expires)
}

pub(super) async fn access(State(panel): State<Panel>, headers: HeaderMap) -> impl IntoResponse {
    Json(access_state(&panel, &headers).await.0)
}

pub(super) async fn setup_password(
    State(panel): State<Panel>,
    Json(setup): Json<SetupPassword>,
) -> Response {
    if !matches!(panel.app.configuration().access(), Access::PendingPassword) {
        return problem(StatusCode::CONFLICT, "setup_complete");
    }
    if !valid_password_length(&setup.password) {
        return problem(StatusCode::BAD_REQUEST, "invalid_password");
    }
    if setup.password != setup.confirmation {
        return problem(StatusCode::BAD_REQUEST, "password_mismatch");
    }
    let result = tokio::task::spawn_blocking(move || {
        let Some(hash) = panel
            .app
            .configuration()
            .set_initial_password(&setup.password)?
        else {
            return Ok::<_, anyhow::Error>(problem(StatusCode::CONFLICT, "setup_complete"));
        };
        // The password is already durable, so a session failure leaves the normal sign-in flow.
        Ok(match create_session(&panel, &hash) {
            Ok(cookie) => session_response(cookie),
            Err(error) => {
                log::error!("Could not create initial panel session: {error:#}");
                StatusCode::NO_CONTENT.into_response()
            }
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => response,
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "password_save_failed"),
    }
}

pub(super) async fn login(State(panel): State<Panel>, Json(login): Json<Login>) -> Response {
    let hash = match panel.app.configuration().access() {
        Access::Open => return StatusCode::NO_CONTENT.into_response(),
        Access::PendingPassword => return problem(StatusCode::CONFLICT, "setup_required"),
        Access::Password { hash } => hash,
    };
    {
        let mut auth = panel.auth.lock().await;
        let now = Instant::now();
        if auth.next_login.is_some_and(|next| now < next) {
            return problem(StatusCode::TOO_MANY_REQUESTS, "login_retry");
        }
        auth.next_login = Some(now + Duration::from_secs(2));
    }
    let result = tokio::task::spawn_blocking(move || {
        if !verify_password(&hash, &login.password) {
            return Ok(None);
        }
        create_session(&panel, &hash).map(Some)
    })
    .await;
    match result {
        Ok(Ok(Some(cookie))) => session_response(cookie),
        Ok(Ok(None)) => problem(StatusCode::UNAUTHORIZED, "wrong_password"),
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

fn session_response(cookie: String) -> Response {
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

fn create_session(panel: &Panel, password_hash: &str) -> Result<String> {
    let token = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
    let mut auth = panel.auth.blocking_lock();
    let now = SystemTime::now();
    let mut sessions = auth.sessions.clone();
    sessions.retain(|_, deadline| *deadline > now);
    if sessions.len() >= 32 {
        let oldest = sessions
            .iter()
            .min_by_key(|(_, deadline)| *deadline)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            sessions.remove(&key);
        }
    }
    sessions.insert(session_key(password_hash, &token), now + SESSION_DURATION);
    auth.save(&panel.app.dir, sessions)?;
    Ok(cookie(&token, SESSION_DURATION.as_secs()))
}

pub(super) async fn logout(
    State(panel): State<Panel>,
    headers: HeaderMap,
    Json(_): Json<serde_json::Value>,
) -> Response {
    if let (Some(token), Access::Password { hash }) =
        (token(&headers), panel.app.configuration().access())
    {
        let key = session_key(&hash, token);
        let result = tokio::task::spawn_blocking(move || {
            let mut auth = panel.auth.blocking_lock();
            if auth.sessions.contains_key(&key) {
                let mut sessions = auth.sessions.clone();
                sessions.remove(&key);
                auth.save(&panel.app.dir, sessions)?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            return problem(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cookie("", 0))],
    )
        .into_response()
}
