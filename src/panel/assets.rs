use axum::{
    Router,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};

use super::Panel;

const INDEX: &[u8] = include_bytes!("../../web/dist/index.html.gz");
const SCRIPT: &[u8] = include_bytes!("../../web/dist/panel.js.gz");
const PLUGIN_RUNTIME: &[u8] = include_bytes!("../../web/dist/plugin-runtime-v1.js.gz");
const STYLE: &[u8] = include_bytes!("../../web/dist/panel.css.gz");
const LICENSES: &[u8] = include_bytes!("../../web/dist/panel-licenses.md.gz");

pub(super) fn routes() -> Router<Panel> {
    let mut router = Router::new().route("/", get(index));
    for path in [
        "/settings",
        "/settings/activation",
        "/settings/assistant",
        "/settings/diagnostics",
        "/settings/panel",
        "/settings/update",
    ] {
        router = router
            .route(path, get(index))
            .route(&format!("{path}/"), get(index));
    }
    router
        .route("/plugins", get(index))
        .route("/plugins/", get(index))
        // Both plugin pages and /plugins/{id}/settings use the panel entry point.
        .route("/plugins/{*path}", get(index))
        .route(
            "/panel.js",
            get(|| async { asset("text/javascript; charset=utf-8", SCRIPT) }),
        )
        .route(
            "/plugin-runtime-v1.js",
            get(|| async { asset("text/javascript; charset=utf-8", PLUGIN_RUNTIME) }),
        )
        .route(
            "/panel.css",
            get(|| async { asset("text/css; charset=utf-8", STYLE) }),
        )
        .route(
            "/panel-licenses.md",
            get(|| async { asset("text/plain; charset=utf-8", LICENSES) }),
        )
}

async fn index() -> Response {
    asset("text/html; charset=utf-8", INDEX)
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
