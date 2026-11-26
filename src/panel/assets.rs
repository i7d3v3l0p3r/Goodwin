use axum::{
    Router,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::panel_core::{ASSET_CACHE_CONTROL, ASSETS, Asset, INDEX, PAGE_PATHS};

use super::Panel;

pub(super) fn routes() -> Router<Panel> {
    let mut router = Router::new().route("/", get(index));
    for &path in PAGE_PATHS.iter().skip(1) {
        router = router
            .route(path, get(index))
            .route(&format!("{path}/"), get(index));
    }
    for asset in ASSETS {
        router = router.route(
            asset.path,
            get(move |headers: HeaderMap| async move { bundle(asset, &headers) }),
        );
    }
    // Both plugin pages and /plugins/{id}/settings use the panel entry point.
    router.route("/plugins/{*path}", get(index))
}

async fn index() -> Response {
    (
        [
            (header::CONTENT_TYPE, INDEX.0),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        INDEX.1,
    )
        .into_response()
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
fn bundle(asset: &Asset, headers: &HeaderMap) -> Response {
    let unchanged = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .any(|tag| asset.cached(tag.to_str().ok()));
    let cache = [
        (header::ETAG, asset.etag),
        (header::CACHE_CONTROL, ASSET_CACHE_CONTROL),
    ];
    if unchanged {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (
        cache,
        [
            (header::CONTENT_TYPE, asset.mime),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        asset.gzip,
    )
        .into_response()
}
