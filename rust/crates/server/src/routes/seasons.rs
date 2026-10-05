use crate::error::AppError;
use crate::*;
use serde_json::json;

pub(crate) async fn get_seasons(State(state): State<SharedState>) -> Handled {
    if state.db.is_none() {
        return Ok(Json(json!([])).into_response());
    }
    let rows = with_db(&state, |conn| db::list_seasons(conn.unwrap()))
        .await
        .map_err(|e| AppError::internal("seasons", e))?;
    Ok(Json(rows).into_response())
}

pub(crate) async fn create_season(db: Db, headers: HeaderMap, body: Bytes) -> Handled {
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    let v = parse_json(&body)?;
    let year = v
        .get("year")
        .and_then(json_int)
        .ok_or_else(|| AppError::bad_request("'year'"))?;
    let sid = db
        .run(move |conn| db::create_season(conn, year))
        .await
        .map_err(|e| AppError::internal("create_season", e))?;
    Ok(Json(json!({ "id": sid, "year": year })).into_response())
}

pub(crate) async fn get_boats(State(state): State<SharedState>) -> Handled {
    if state.db.is_none() {
        return Ok(Json(json!([])).into_response());
    }
    let rows = with_db(&state, |conn| db::list_boats(conn.unwrap()))
        .await
        .map_err(|e| AppError::internal("boats", e))?;
    Ok(Json(rows).into_response())
}

pub(crate) async fn rename_boat(db: Db, body: Bytes) -> Handled {
    let v = parse_json(&body)?;
    let boat_id = v
        .get("id")
        .and_then(json_int)
        .ok_or_else(|| AppError::bad_request("'id'"))?;
    // Empty or missing name clears the custom name (stored as NULL).
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    db.run(move |conn| db::rename_boat(conn, boat_id, name.as_deref()))
        .await
        .map_err(|e| AppError::internal("rename_boat", e))?;
    Ok(ok_true())
}
