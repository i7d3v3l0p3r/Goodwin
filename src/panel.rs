mod assets;
mod auth;
mod plugin_routes;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{any, delete, get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::{
    app::{App, Failure, Source},
    config::{Settings, normalize_system_prompt},
};

use auth::{
    Auth, access, login, logout, require_owner, require_password_owner, require_same_origin,
    response_headers, setup_password,
};
use plugin_routes::{plugin_asset, plugin_catalog, plugin_http, remove_plugin, upload_plugin};

#[derive(Clone)]
struct Panel {
    app: Arc<App>,
    auth: Arc<Mutex<Auth>>,
}

pub fn router(app: Arc<App>) -> Router {
    let auth = if app.configuration().password_protected() {
        Auth::load(&app.dir)
    } else {
        Auth::default()
    };
    let panel = Panel {
        app,
        auth: Arc::new(Mutex::new(auth)),
    };
    let protected = Router::new()
        .route("/api/state", get(snapshot))
        .route("/api/settings", put(settings))
        .route("/api/settings/system-prompt", put(system_prompt))
        .route("/api/chat", post(chat))
        .route("/api/chat/clear", post(clear))
        .route("/api/test", post(test))
        .route("/api/plugins", get(plugin_catalog))
        .route("/api/plugins/{id}/{*path}", any(plugin_http))
        .route("/plugin-assets/{id}/{revision}/{*path}", get(plugin_asset))
        .route_layer(middleware::from_fn_with_state(panel.clone(), require_owner));

    let packages = Router::new()
        .route("/api/plugin-packages", post(upload_plugin))
        .route("/api/plugin-packages/{id}", delete(remove_plugin))
        .route_layer(middleware::from_fn_with_state(
            panel.clone(),
            require_password_owner,
        ));

    assets::routes()
        .route("/api/access", get(access))
        .route("/api/setup", post(setup_password))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .merge(protected)
        .merge(packages)
        .layer(DefaultBodyLimit::max(16_384))
        .layer(middleware::from_fn(response_headers))
        .layer(middleware::from_fn(require_same_origin))
        .with_state(panel)
}

async fn snapshot(State(panel): State<Panel>) -> impl IntoResponse {
    Json(panel.app.snapshot())
}

#[derive(Deserialize)]
struct UpdateSettings {
    settings: Settings,
    key: Option<String>,
}

async fn settings(State(panel): State<Panel>, Json(update): Json<UpdateSettings>) -> Response {
    let UpdateSettings { mut settings, key } = update;
    if settings.validate().is_err() {
        return problem(StatusCode::BAD_REQUEST, "invalid_settings");
    }
    let key = key.map(|value| value.trim().to_owned());
    if key.as_ref().is_some_and(|value| {
        value.len() > 512 || !value.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return problem(StatusCode::BAD_REQUEST, "invalid_settings");
    }
    let app = Arc::clone(&panel.app);
    let saved =
        tokio::task::spawn_blocking(move || app.configuration().update_settings(settings, key))
            .await;
    match saved {
        Ok(Ok(())) => snapshot(State(panel)).await.into_response(),
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "save_failed"),
    }
}

#[derive(Deserialize)]
struct UpdateSystemPrompt {
    system_prompt: String,
}

async fn system_prompt(
    State(panel): State<Panel>,
    Json(update): Json<UpdateSystemPrompt>,
) -> Response {
    let Ok(prompt) = normalize_system_prompt(&update.system_prompt) else {
        return problem(StatusCode::BAD_REQUEST, "invalid_system_prompt");
    };
    let saved =
        tokio::task::spawn_blocking(move || panel.app.configuration().update_system_prompt(prompt))
            .await;
    match saved {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        _ => problem(StatusCode::INTERNAL_SERVER_ERROR, "save_failed"),
    }
}

#[derive(Deserialize)]
struct Chat {
    message: String,
}

async fn chat(State(panel): State<Panel>, Json(chat): Json<Chat>) -> Response {
    let message = chat.message.trim().to_owned();
    if message.is_empty() || message.len() > 8_000 || message.chars().count() > 2000 {
        return problem(StatusCode::BAD_REQUEST, "invalid_message");
    }
    let id = match panel.app.start(Source::Chat) {
        Ok(id) => id,
        Err(error) => return blocked(error),
    };
    tokio::spawn(async move {
        panel.app.chat(id, message).await;
    });
    StatusCode::ACCEPTED.into_response()
}

async fn clear(State(panel): State<Panel>, Json(_): Json<serde_json::Value>) -> Response {
    match panel.app.clear_chat() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => blocked(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TestMode {
    Manual,
    Wake,
}

#[derive(Deserialize)]
struct Test {
    mode: TestMode,
}

async fn test(State(panel): State<Panel>, Json(test): Json<Test>) -> Response {
    let source = match test.mode {
        TestMode::Manual => Source::ManualTest,
        TestMode::Wake => Source::WakeTest,
    };
    match panel.app.start(source) {
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(error) => blocked(error),
    }
}

fn blocked(error: Failure) -> Response {
    (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
}

fn problem(status: StatusCode, error: &'static str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}
