mod assets;
mod auth;
mod diagnostics;
mod events;
mod json_body;
mod plugin_routes;
mod updates;
mod wake_models;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{any, delete, get, post, put},
};
use goodwin_error::Context;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::{
    app::{App, Failure, Source},
    config::{Settings, normalize_system_prompt},
    panel_core::{Chat, UpdateMicrophone, UpdateSystemPrompt},
    plugins::SETTINGS_BODY_LIMIT,
};

use auth::{
    Auth, access, login, logout, require_owner, require_password_owner, require_same_origin,
    response_headers, setup_password,
};
use json_body::{Json, JsonDocument};
use plugin_routes::{
    plugin_asset, plugin_catalog, plugin_http, plugin_models, plugin_settings,
    provider_connections, remove_plugin, remove_provider_connection, save_provider_connection,
    update_plugin_settings, upload_plugin,
};

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
        .route("/api/diagnostics/resources", get(diagnostics::resources))
        .route("/api/diagnostics/logs", get(diagnostics::logs))
        .route(
            "/api/core-update",
            get(updates::status).post(updates::install),
        )
        .route("/api/core-update/latest", get(updates::latest))
        .route("/api/settings", put(settings))
        .route("/api/settings/microphone", put(microphone))
        .route("/api/settings/system-prompt", put(system_prompt))
        .route(
            "/api/wake-models",
            get(wake_models::catalog).put(wake_models::install),
        )
        .route("/api/chat", post(chat))
        .route("/api/chat/clear", post(clear))
        .route("/api/test", post(test))
        .route("/api/voice/cancel", post(cancel_voice))
        .route("/api/plugins", get(plugin_catalog))
        .route("/api/plugin-models/{id}/{capability}", get(plugin_models))
        .route(
            "/api/plugin-connections/{id}",
            get(provider_connections).put(save_provider_connection),
        )
        .route(
            "/api/plugin-connections/{id}/{connection}",
            delete(remove_provider_connection),
        )
        .route(
            "/api/plugin-settings/{id}",
            get(plugin_settings)
                .patch(update_plugin_settings)
                .layer(DefaultBodyLimit::max(SETTINGS_BODY_LIMIT)),
        )
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
        .route(
            "/api/capabilities",
            get(|| async { Json(crate::model::capabilities()) }),
        )
        .route("/api/access", get(access))
        .route("/api/events", get(events::subscribe))
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

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
async fn snapshot(State(panel): State<Panel>) -> impl IntoResponse {
    Json(panel.app.snapshot())
}

#[derive(Deserialize)]
struct UpdateSettings {
    settings: Settings,
}

async fn settings(State(panel): State<Panel>, Json(update): Json<UpdateSettings>) -> Response {
    let UpdateSettings { settings } = update;
    if settings.validate().is_err() {
        return problem(StatusCode::BAD_REQUEST, "invalid_settings");
    }
    let app = Arc::clone(&panel.app);
    let saved =
        tokio::task::spawn_blocking(move || app.configuration().update_settings(settings)).await;
    match saved
        .context("Settings writer task")
        .and_then(|saved| saved)
    {
        Ok(()) => snapshot(State(panel)).await.into_response(),
        Err(error) => {
            log::error!("Could not save settings: {error:#}");
            problem(StatusCode::INTERNAL_SERVER_ERROR, "save_failed")
        }
    }
}

async fn microphone(State(panel): State<Panel>, Json(update): Json<UpdateMicrophone>) -> Response {
    let saved =
        tokio::task::spawn_blocking(move || panel.app.set_microphone_muted(update.muted)).await;
    match saved
        .context("Microphone settings writer task")
        .and_then(|saved| saved)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            log::error!("Could not save microphone settings: {error:#}");
            problem(StatusCode::INTERNAL_SERVER_ERROR, "save_failed")
        }
    }
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
    match saved
        .context("System prompt writer task")
        .and_then(|saved| saved)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            log::error!("Could not save the system prompt: {error:#}");
            problem(StatusCode::INTERNAL_SERVER_ERROR, "save_failed")
        }
    }
}

async fn chat(State(panel): State<Panel>, Json(chat): Json<Chat>) -> Response {
    let test = chat.test;
    let Some(message) = chat.into_message() else {
        return problem(StatusCode::BAD_REQUEST, "invalid_message");
    };
    let started = if test {
        panel.app.start_chat_test()
    } else {
        panel.app.start(Source::Chat)
    };
    let id = match started {
        Ok(id) => id,
        Err(error) => return blocked(error),
    };
    tokio::spawn(async move {
        panel.app.chat(id, message).await;
    });
    StatusCode::ACCEPTED.into_response()
}

async fn clear(State(panel): State<Panel>, _: JsonDocument) -> Response {
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
        Err(error) => {
            log::warn!("Voice test could not start source={source:?} error={error:?}");
            blocked(error)
        }
    }
}

async fn cancel_voice(
    State(panel): State<Panel>,
    Json(cancel): Json<crate::panel_core::CancelVoice>,
) -> Response {
    if panel.app.cancel_voice(cancel.request_id) {
        StatusCode::ACCEPTED.into_response()
    } else {
        problem(StatusCode::CONFLICT, "voice_request_inactive")
    }
}

fn blocked(error: Failure) -> Response {
    (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response()
}

fn problem(status: StatusCode, error: &'static str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}
