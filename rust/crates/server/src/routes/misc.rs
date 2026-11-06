use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Static + config
// ---------------------------------------------------------------------------

pub(crate) async fn root(headers: HeaderMap) -> Response {
    let etag = HeaderValue::from_static(VIEWER_ETAG);
    let cache = (header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if headers.get(header::IF_NONE_MATCH) == Some(&etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag), cache]).into_response();
    }
    ([(header::ETAG, etag), cache], Html(VIEWER_HTML)).into_response()
}

// The service worker must be served from the root to control every page, and
// with no-cache so a new release is picked up on the next visit.
pub(crate) async fn service_worker() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../../static/sw.js"),
    )
        .into_response()
}

pub(crate) async fn manifest() -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        include_str!("../../static/manifest.webmanifest"),
    )
        .into_response()
}

pub(crate) async fn icon() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../../static/icon.svg"),
    )
        .into_response()
}

pub(crate) async fn config_endpoint(State(state): State<SharedState>) -> Response {
    Json(json!({
        "boats": state.cfg.boats,
        "levels": state.cfg.levels,
        "restricted": state.cfg.restricted,
        "diverse": state.cfg.diverse,
        "adj": state.cfg.adj,
        "db": state.db.is_some(),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

pub(crate) async fn api_health() -> Response {
    Json(json!({
        "status": "healthy",
        "timestamp": utc_now_iso(),
        "version": "1.0",
    }))
    .into_response()
}
