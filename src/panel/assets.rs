use axum::{
    Router,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};

use super::Panel;

const INDEX: &str = include_str!("../../web/dist/index.html");
const SCRIPT: &str = include_str!("../../web/dist/panel.js");
const PLUGIN_RUNTIME: &str = include_str!("../../web/dist/plugin-runtime-v1.js");
const STYLE: &str = include_str!("../../web/dist/panel.css");
const LICENSES: &str = include_str!("../../web/dist/panel-licenses.md");

pub(super) fn routes() -> Router<Panel> {
    let mut router = Router::new().route("/", get(index));
    for path in [
        "/settings",
        "/settings/activation",
        "/settings/assistant",
        "/settings/diagnostics",
        "/settings/panel",
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

fn asset(mime: &'static str, content: &'static str) -> Response {
    ([(header::CONTENT_TYPE, mime)], content).into_response()
}
