use axum::{
    Json,
    body::{Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use goodwin_plugin::provider::Capability;
use serde_json::{Map, Value};

use crate::plugins::{ARCHIVE_LIMIT, Failure as PluginFailure};

use super::{Panel, problem};

pub(super) async fn plugin_catalog(State(panel): State<Panel>) -> impl IntoResponse {
    Json(panel.app.plugins.catalog())
}

pub(super) async fn plugin_models(
    State(panel): State<Panel>,
    Path((id, capability)): Path<(String, Capability)>,
) -> Response {
    match panel.app.plugins.models(&id, capability).await {
        Ok(catalog) => Json(catalog).into_response(),
        Err(error) => plugin_problem(error),
    }
}

pub(super) async fn plugin_settings(
    State(panel): State<Panel>,
    Path(id): Path<String>,
) -> Response {
    match panel.app.plugins.settings(&id).await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => plugin_problem(error),
    }
}

pub(super) async fn update_plugin_settings(
    State(panel): State<Panel>,
    Path(id): Path<String>,
    Json(fields): Json<Map<String, Value>>,
) -> Response {
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.update_settings(&id, fields).await }).await {
        Ok(Ok(settings)) => Json(settings).into_response(),
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
    }
}

pub(super) async fn upload_plugin(State(panel): State<Panel>, request: Request) -> Response {
    let Ok(bytes) = to_bytes(request.into_body(), ARCHIVE_LIMIT).await else {
        return problem(StatusCode::PAYLOAD_TOO_LARGE, "plugin_archive_too_large");
    };
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.upload(bytes).await }).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
    }
}

pub(super) async fn remove_plugin(State(panel): State<Panel>, Path(id): Path<String>) -> Response {
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.remove(&id).await }).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
    }
}

pub(super) async fn plugin_http(
    State(panel): State<Panel>,
    Path((id, path)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, 16_384).await else {
        return problem(StatusCode::PAYLOAD_TOO_LARGE, "invalid_plugin_request");
    };
    let body = if body.is_empty() {
        Value::Null
    } else {
        let Ok(value) = serde_json::from_slice(&body) else {
            return problem(StatusCode::BAD_REQUEST, "invalid_plugin_request");
        };
        value
    };
    match panel
        .app
        .plugins
        .http(
            &id,
            parts.method.as_str(),
            &format!("/{path}"),
            parts.uri.query(),
            body,
        )
        .await
    {
        Ok(response) => {
            let Ok(status) = StatusCode::from_u16(response.status) else {
                return plugin_problem(PluginFailure::Unavailable);
            };
            if !(status.is_success() || status.is_client_error() || status.is_server_error()) {
                return plugin_problem(PluginFailure::Unavailable);
            }
            match response.body {
                Some(body) if status != StatusCode::NO_CONTENT => {
                    (status, Json(body)).into_response()
                }
                _ => status.into_response(),
            }
        }
        Err(error) => plugin_problem(error),
    }
}

pub(super) async fn plugin_asset(
    State(panel): State<Panel>,
    Path((id, revision, path)): Path<(String, String, String)>,
) -> Response {
    let mime = match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("mjs" | "js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("jpeg" | "jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => return plugin_problem(PluginFailure::NotFound),
    };
    let plugins = panel.app.plugins.clone();
    match tokio::task::spawn_blocking(move || plugins.asset(&id, &revision, &path)).await {
        Ok(Some(bytes)) => ([(header::CONTENT_TYPE, mime)], Bytes::from(bytes)).into_response(),
        _ => plugin_problem(PluginFailure::NotFound),
    }
}

fn plugin_problem(error: PluginFailure) -> Response {
    let status = match error {
        PluginFailure::InvalidPackage | PluginFailure::InvalidSettings => StatusCode::BAD_REQUEST,
        PluginFailure::NotFound => StatusCode::NOT_FOUND,
        PluginFailure::StartFailed | PluginFailure::RolledBack => StatusCode::CONFLICT,
        PluginFailure::Storage => StatusCode::INTERNAL_SERVER_ERROR,
        PluginFailure::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    problem(status, error.code())
}
