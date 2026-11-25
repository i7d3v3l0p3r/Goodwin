use axum::{
    Router,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};

use crate::panel_core::{ASSETS, INDEX, PAGE_PATHS};

use super::Panel;

pub(super) fn routes() -> Router<Panel> {
    let mut router = Router::new().route("/", get(index));
    for &path in PAGE_PATHS.iter().skip(1) {
        router = router
            .route(path, get(index))
            .route(&format!("{path}/"), get(index));
    }
    for &(path, mime, content) in ASSETS {
        router = router.route(path, get(move || async move { asset(mime, content) }));
    }
    // Both plugin pages and /plugins/{id}/settings use the panel entry point.
    router.route("/plugins/{*path}", get(index))
}

async fn index() -> Response {
    asset(INDEX.0, INDEX.1)
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
fn asset(mime: &'static str, content: &'static [u8]) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        content,
    )
        .into_response()
}
