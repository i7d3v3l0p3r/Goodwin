use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use crate::core_update;

use super::{Json, Panel, problem};

type UpdateError = (StatusCode, &'static str);

fn status_for(panel: &Panel) -> Result<core_update::Status, UpdateError> {
    core_update::status(&panel.app.dir).map_err(|error| {
        log::warn!("Could not read Goodwin update status: {error:#}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "core_update_status_failed",
        )
    })
}

pub(super) async fn status(State(panel): State<Panel>) -> Response {
    match status_for(&panel) {
        Ok(status) => Json(status).into_response(),
        Err((status, code)) => problem(status, code),
    }
}

fn update_available(panel: &Panel) -> Result<(), UpdateError> {
    if !panel.app.configuration().password_protected() {
        return Err((StatusCode::FORBIDDEN, "core_update_password_required"));
    }
    if !status_for(panel)?.supported {
        return Err((StatusCode::CONFLICT, "core_update_unsupported"));
    }
    Ok(())
}

pub(super) async fn latest(State(panel): State<Panel>) -> Response {
    if let Err((status, code)) = update_available(&panel) {
        return problem(status, code);
    }
    match tokio::task::spawn_blocking(core_update::latest).await {
        Ok(Ok(version)) => Json(json!({
            "version": version,
            "available": core_update::newer(&version, core_update::VERSION),
        }))
        .into_response(),
        Ok(Err(error)) => {
            log::warn!("Could not check for a Goodwin release: {error:#}");
            problem(StatusCode::BAD_GATEWAY, "core_update_check_failed")
        }
        Err(_) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "core_update_check_failed",
        ),
    }
}

#[derive(Deserialize)]
pub(super) struct Install {
    version: String,
}

pub(super) async fn install(State(panel): State<Panel>, Json(install): Json<Install>) -> Response {
    if let Err((status, code)) = update_available(&panel) {
        return problem(status, code);
    }
    if !core_update::newer(&install.version, core_update::VERSION) {
        return problem(StatusCode::BAD_REQUEST, "core_update_invalid_version");
    }
    match core_update::start(install.version).await {
        Ok(true) => StatusCode::ACCEPTED.into_response(),
        Ok(false) => problem(StatusCode::CONFLICT, "core_update_busy"),
        Err(error) => {
            log::error!("Could not start the Goodwin updater: {error:#}");
            problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "core_update_start_failed",
            )
        }
    }
}
