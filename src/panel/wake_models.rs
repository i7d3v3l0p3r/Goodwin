use std::{sync::Arc, time::Duration};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;

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
    match tokio::task::spawn_blocking(move || app.wake_models.install(&install.tag, &install.id))
        .await
    {
        Ok(Ok((selection, activated))) => match timeout(Duration::from_secs(15), activated).await {
            Ok(Ok(())) => Json(selection).into_response(),
            _ => problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "wake_model_activation_pending",
            ),
        },
        Ok(Err(InstallError::Busy)) => problem(StatusCode::CONFLICT, "wake_model_busy"),
        Ok(Err(InstallError::CatalogChanged)) => {
            problem(StatusCode::CONFLICT, "wake_catalog_changed")
        }
        Ok(Err(InstallError::Unknown)) => problem(StatusCode::NOT_FOUND, "invalid_wake_model"),
        Ok(Err(InstallError::Unavailable(error))) => {
            log::warn!("Could not get the wake model catalog: {error:#}");
            problem(StatusCode::BAD_GATEWAY, "wake_catalog_unavailable")
        }
        Ok(Err(InstallError::Failed(error))) => {
            log::error!("Could not install the wake model: {error:#}");
            problem(StatusCode::INTERNAL_SERVER_ERROR, "wake_model_failed")
        }
        Err(_) => problem(StatusCode::INTERNAL_SERVER_ERROR, "wake_model_failed"),
    }
}
