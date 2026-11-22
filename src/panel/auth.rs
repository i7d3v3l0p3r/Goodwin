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
use serde::Deserialize;
use serde_json::json;

use crate::config::{Access, random_bytes, verify_password};

use super::{Panel, problem};

#[cfg(test)]
mod tests;

const SESSION_DURATION: Duration = Duration::from_secs(12 * 60 * 60);
const COOKIE: &str = "goodwin_session";
const SESSION_FILE: &str = "panel-sessions.json";

#[derive(Default)]
pub(super) struct Auth {
    sessions: HashMap<String, SystemTime>,
    next_login: Option<Instant>,
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
                eprintln!("Could not restore panel sessions: {error:#}");
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
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
        "default-src 'self'; font-src 'self' data:; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
    ));
    response
}

pub(super) async fn require_same_origin(request: axum::extract::Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/api/") {
        return next.run(request).await;
    }
    let headers = request.headers();
    let cross_site = headers
        .get("sec-fetch-site")
        .is_some_and(|value| value == "cross-site");
    let wrong_origin = headers.get(header::ORIGIN).is_some_and(|value| {
        let origin = value.to_str().unwrap_or_default();
        let host = headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .or_else(|| {
                request
                    .uri()
                    .authority()
                    .map(|authority| authority.as_str())
            });
        origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            != host
            || host.is_none()
    });
    if cross_site || wrong_origin {
        return problem(StatusCode::FORBIDDEN, "origin");
    }
    next.run(request).await
}

fn token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == COOKIE).then_some(value))
}

async fn authorized(panel: &Panel, headers: &HeaderMap) -> bool {
    match panel.app.configuration().access() {
        Access::Open => true,
        Access::Password { hash } => has_session(panel, headers, &hash).await,
    }
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
    let Some(token) = token(headers) else {
        return false;
    };
    let key = session_key(password_hash, token);
    panel
        .auth
        .lock()
        .await
        .sessions
        .get(&key)
        .is_some_and(|expiry| SystemTime::now() < *expiry)
}

pub(super) async fn require_password_owner(
    State(panel): State<Panel>,
    request: Request,
    next: Next,
) -> Response {
    let Access::Password { hash } = panel.app.configuration().access() else {
        return problem(StatusCode::FORBIDDEN, "plugin_password_required");
    };
    if !has_session(&panel, request.headers(), &hash).await {
        return problem(StatusCode::UNAUTHORIZED, "login_required");
    }
    next.run(request).await
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

pub(super) async fn access(State(panel): State<Panel>, headers: HeaderMap) -> impl IntoResponse {
    let (protected, authorized) = match panel.app.configuration().access() {
        Access::Open => (false, true),
        Access::Password { hash } => (true, has_session(&panel, &headers, &hash).await),
    };
    Json(json!({ "protected": protected, "authorized": authorized }))
}

#[derive(Deserialize)]
pub(super) struct Login {
    password: String,
}

pub(super) async fn login(State(panel): State<Panel>, Json(login): Json<Login>) -> Response {
    let access = panel.app.configuration().access();
    let Access::Password { hash } = access else {
        return StatusCode::NO_CONTENT.into_response();
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
        Ok(Ok(Some(cookie))) => {
            (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
        }
        Ok(Ok(None)) => problem(StatusCode::UNAUTHORIZED, "wrong_password"),
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
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

fn cookie(token: &str, age: u64) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}")
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
