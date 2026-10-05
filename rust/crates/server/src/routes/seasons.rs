use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Seasons
// ---------------------------------------------------------------------------

pub(crate) async fn get_seasons(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_seasons(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("seasons", e),
    }
}

pub(crate) async fn create_season(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database configured");
    }
    if !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let year = match v.get("year").and_then(json_int) {
        Some(y) => y,
        None => return e_error(StatusCode::BAD_REQUEST, "'year'"),
    };
    match with_db(&state, move |conn| db::create_season(conn.unwrap(), year)).await {
        Ok(sid) => Json(json!({ "id": sid, "year": year })).into_response(),
        Err(e) => server_error("create_season", e),
    }
}

// ---------------------------------------------------------------------------
// Boats
// ---------------------------------------------------------------------------

pub(crate) async fn get_boats(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_boats(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("boats", e),
    }
}

pub(crate) async fn rename_boat(State(state): State<SharedState>, body: Bytes) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let boat_id = match v.get("id").and_then(json_int) {
        Some(b) => b,
        None => return e_error(StatusCode::BAD_REQUEST, "'id'"),
    };
    // `body.get("name") or None` → empty/missing becomes NULL.
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    match with_db(&state, move |conn| {
        db::rename_boat(conn.unwrap(), boat_id, name.as_deref())
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("rename_boat", e),
    }
}
