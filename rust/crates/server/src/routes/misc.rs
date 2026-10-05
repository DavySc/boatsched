use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Static + config
// ---------------------------------------------------------------------------

pub(crate) async fn root() -> Html<&'static str> {
    Html(VIEWER_HTML)
}

pub(crate) async fn config_endpoint(State(state): State<SharedState>) -> Response {
    Json(json!({
        "boats": state.cfg.boats,
        "levels": state.cfg.levels,
        "restricted": state.cfg.restricted,
        "diverse": state.cfg.diverse,
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
