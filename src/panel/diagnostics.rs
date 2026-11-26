use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::journal;

use super::{Json, Panel, auth::password_rejection};

#[cfg(target_os = "linux")]
mod linux;

#[derive(Default, Serialize)]
struct Resources {
    supported: bool,
    cpu_percent: Option<f64>,
    memory: Option<Usage>,
    storage: Option<Usage>,
    uptime_seconds: Option<u64>,
    goodwin_cpu_percent: Option<f64>,
    goodwin_rss_bytes: Option<u64>,
}

#[derive(Serialize)]
struct Usage {
    used_bytes: u64,
    total_bytes: u64,
}

#[derive(Serialize)]
struct Logs {
    supported: bool,
    kernel: Option<String>,
    goodwin: Option<String>,
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
pub(super) async fn resources(State(panel): State<Panel>) -> impl IntoResponse {
    #[cfg(target_os = "linux")]
    let data = linux::resources(panel.app.dir.clone()).await;

    #[cfg(not(target_os = "linux"))]
    let data = {
        drop(panel);
        Resources::default()
    };
    Json(data)
}

pub(super) async fn metrics() -> impl IntoResponse {
    Json(crate::diagnostics::snapshot())
}

pub(super) async fn logs(State(panel): State<Panel>, headers: HeaderMap) -> Response {
    if let Some(response) =
        password_rejection(&panel, &headers, "diagnostics_password_required").await
    {
        return response;
    }

    let dir = panel.app.dir.clone();
    let goodwin = tokio::task::spawn_blocking(move || journal::recent(&dir));
    #[cfg(target_os = "linux")]
    let (kernel, goodwin) = tokio::join!(linux::read_kernel_log(), goodwin);
    #[cfg(not(target_os = "linux"))]
    let (kernel, goodwin) = (None, goodwin.await);
    let goodwin = goodwin.ok().flatten();
    Json(Logs {
        supported: cfg!(target_os = "linux") || goodwin.is_some(),
        kernel,
        goodwin,
    })
    .into_response()
}
