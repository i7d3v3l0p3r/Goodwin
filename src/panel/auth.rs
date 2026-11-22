use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::json;

use crate::config::{Access, random_bytes, verify_password};

use super::{Panel, problem};

#[cfg(test)]
mod tests;

const SESSION_DURATION: Duration = Duration::from_secs(12 * 60 * 60);
const COOKIE: &str = "goodwin_session";

#[derive(Default)]
pub(super) struct Auth {
    sessions: HashMap<String, Instant>,
    next_login: Option<Instant>,
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

fn authorized(panel: &Panel, headers: &HeaderMap) -> bool {
    if !panel.app.configuration().password_protected() {
        return true;
    }
    has_session(panel, headers)
}

fn has_session(panel: &Panel, headers: &HeaderMap) -> bool {
    let Some(token) = token(headers) else {
        return false;
    };
    panel
        .auth
        .lock()
        .expect("session mutex")
        .sessions
        .get(token)
        .is_some_and(|expiry| Instant::now() < *expiry)
}

pub(super) async fn require_password_owner(
    State(panel): State<Panel>,
    request: Request,
    next: Next,
) -> Response {
    if !panel.app.configuration().password_protected() {
        return problem(StatusCode::FORBIDDEN, "plugin_password_required");
    }
    if !has_session(&panel, request.headers()) {
        return problem(StatusCode::UNAUTHORIZED, "login_required");
    }
    next.run(request).await
}

pub(super) async fn require_owner(
    State(panel): State<Panel>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if authorized(&panel, request.headers()) {
        next.run(request).await
    } else {
        problem(StatusCode::UNAUTHORIZED, "login_required")
    }
}

pub(super) async fn access(State(panel): State<Panel>, headers: HeaderMap) -> impl IntoResponse {
    let protected = panel.app.configuration().password_protected();
    Json(json!({ "protected": protected, "authorized": authorized(&panel, &headers) }))
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
        let mut auth = panel.auth.lock().expect("session mutex");
        let now = Instant::now();
        if auth.next_login.is_some_and(|next| now < next) {
            return problem(StatusCode::TOO_MANY_REQUESTS, "login_retry");
        }
        auth.next_login = Some(now + Duration::from_secs(2));
    }
    let matched = tokio::task::spawn_blocking(move || verify_password(&hash, &login.password))
        .await
        .unwrap_or(false);
    if !matched {
        return problem(StatusCode::UNAUTHORIZED, "wrong_password");
    }
    match create_session(&panel) {
        Ok(cookie) => (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response(),
        Err(_) => problem(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

fn create_session(panel: &Panel) -> Result<String> {
    let token = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
    let now = Instant::now();
    let mut auth = panel.auth.lock().expect("session mutex");
    auth.sessions.retain(|_, deadline| *deadline > now);
    if auth.sessions.len() >= 32 {
        let oldest = auth
            .sessions
            .iter()
            .min_by_key(|(_, deadline)| *deadline)
            .map(|(token, _)| token.clone());
        if let Some(token) = oldest {
            auth.sessions.remove(&token);
        }
    }
    auth.sessions.insert(token.clone(), now + SESSION_DURATION);
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
    if let Some(token) = token(&headers) {
        panel
            .auth
            .lock()
            .expect("session mutex")
            .sessions
            .remove(token);
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cookie("", 0))],
    )
        .into_response()
}
