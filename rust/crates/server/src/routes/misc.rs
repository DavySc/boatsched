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
