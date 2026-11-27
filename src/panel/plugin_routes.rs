use axum::{
    Json as AxumJson,
    body::{Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use goodwin_plugin::{HttpRequest, provider::Capability};
use serde_json::{Map, Value};

use crate::{
    config::Access,
    plugins::{ARCHIVE_LIMIT, ConnectionUpdate, Failure as PluginFailure, package::asset_mime},
};

use super::{Json, Panel, problem};

pub(super) async fn plugin_catalog(State(panel): State<Panel>) -> impl IntoResponse {
    Json(panel.app.plugins.catalog())
}

#[cfg(target_os = "linux")]
pub(super) async fn route_via_vpn(
    State(panel): State<Panel>,
    Path(id): Path<String>,
    Json(selected): Json<bool>,
) -> Response {
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.route_via_vpn(&id, selected).await }).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
    }
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

pub(super) async fn provider_connections(
    State(panel): State<Panel>,
    Path(id): Path<String>,
) -> Response {
    match panel.app.plugins.connections(&id).await {
        Ok(connections) => Json(connections).into_response(),
        Err(error) => plugin_problem(error),
    }
}

pub(super) async fn save_provider_connection(
    State(panel): State<Panel>,
    Path(id): Path<String>,
    Json(update): Json<ConnectionUpdate>,
) -> Response {
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.save_connection(&id, update).await }).await {
        Ok(Ok((id, connections))) => {
            Json(serde_json::json!({"id": id, "connections": connections})).into_response()
        }
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
    }
}

pub(super) async fn remove_provider_connection(
    State(panel): State<Panel>,
    Path((id, connection)): Path<(String, String)>,
) -> Response {
    let plugins = panel.app.plugins.clone();
    match tokio::spawn(async move { plugins.remove_connection(&id, connection).await }).await {
        Ok(Ok(connections)) => Json(connections).into_response(),
        Ok(Err(error)) => plugin_problem(error),
        Err(_) => plugin_problem(PluginFailure::Storage),
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
    AxumJson(fields): AxumJson<Map<String, Value>>,
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
            HttpRequest {
                method: parts.method.to_string(),
                path: format!("/{path}"),
                query: parts.uri.query().map(str::to_owned),
                body,
            },
            matches!(panel.app.configuration().access(), Access::Password { .. }),
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
    let Some(mime) = asset_mime(&path) else {
        return plugin_problem(PluginFailure::NotFound);
    };
    let plugins = panel.app.plugins.clone();
    match tokio::task::spawn_blocking(move || plugins.asset(&id, &revision, &path)).await {
        Ok(Some(bytes)) => ([(header::CONTENT_TYPE, mime)], Bytes::from(bytes)).into_response(),
        _ => plugin_problem(PluginFailure::NotFound),
    }
}

fn plugin_problem(error: PluginFailure) -> Response {
    let status = StatusCode::from_u16(error.status()).expect("valid plugin error HTTP status");
    problem(status, error.code())
}
