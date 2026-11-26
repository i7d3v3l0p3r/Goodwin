use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    wake::{InstallError, runtimes},
    wake_catalog::{Catalog, valid_model_id},
};

use super::{Json, Panel, problem};

pub(super) async fn catalog() -> Response {
    match tokio::task::spawn_blocking(|| Catalog::latest(runtimes())).await {
        Ok(Ok(catalog)) => Json(json!({
            "tag": catalog.tag,
            "models": catalog.models.into_iter().map(|model| model.selection).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(error)) => {
            log::warn!("Could not get the wake model catalog: {error:#}");
            problem(StatusCode::BAD_GATEWAY, "wake_catalog_unavailable")
        }
        Err(_) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "wake_catalog_unavailable",
        ),
    }
}

#[derive(Deserialize)]
pub(super) struct Install {
    id: String,
    tag: String,
}

pub(super) async fn install(State(panel): State<Panel>, Json(install): Json<Install>) -> Response {
    if !valid_model_id(&install.id) || install.tag.len() > 33 {
        return problem(StatusCode::BAD_REQUEST, "invalid_wake_model");
    }
    let app = Arc::clone(&panel.app);
    match tokio::task::spawn_blocking(move || {
        let operation = app.wake_models.start_install()?;
        tokio::spawn(async move {
            let worker = Arc::clone(&app);
            if let Err(error) = tokio::task::spawn_blocking(move || {
                worker.wake_models.install(&install.tag, &install.id);
            })
            .await
            {
                log::error!("Wake model installation task failed: {error}");
                app.wake_models.fail_install(InstallError::Failed);
            }
        });
        Ok(operation)
    })
    .await
    {
        Ok(Ok(operation)) => {
            (StatusCode::ACCEPTED, Json(json!({"operation": operation}))).into_response()
        }
        Ok(Err(error)) => install_problem(error),
        Err(_) => install_problem(InstallError::Failed),
    }
}

#[derive(Deserialize)]
pub(super) struct InstallTicket {
    operation: String,
}

pub(super) async fn install_result(
    State(panel): State<Panel>,
    Json(ticket): Json<InstallTicket>,
) -> Response {
    match panel.app.wake_models.install_result(&ticket.operation) {
        Some(Ok(Some(selection))) => Json(selection).into_response(),
        Some(Ok(None)) => (StatusCode::ACCEPTED, Json(json!({"pending": true}))).into_response(),
        Some(Err(error)) => install_problem(error),
        None => problem(StatusCode::NOT_FOUND, "wake_model_failed"),
    }
}

fn install_problem(error: InstallError) -> Response {
    let (status, code) = match error {
        InstallError::Busy => (StatusCode::CONFLICT, "wake_model_busy"),
        InstallError::CatalogChanged => (StatusCode::CONFLICT, "wake_catalog_changed"),
        InstallError::Unknown => (StatusCode::NOT_FOUND, "invalid_wake_model"),
        InstallError::Unavailable => (StatusCode::BAD_GATEWAY, "wake_catalog_unavailable"),
        InstallError::Failed => (StatusCode::INTERNAL_SERVER_ERROR, "wake_model_failed"),
        InstallError::Interrupted => (StatusCode::CONFLICT, "wake_model_interrupted"),
    };
    problem(status, code)
}
