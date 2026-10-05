use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Skill catalog + student progress (evaluation cards). All require a logged-in
// instructor (gated by `auth_middleware`); none require admin — recording an
// evaluation is an instructor activity.
// ---------------------------------------------------------------------------

/// The seeded skill catalog, grouped by level. Source of truth for the eval card.
pub(crate) async fn get_skills(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    match with_db(&state, |conn| db::list_skill_catalog(conn.unwrap())).await {
        Ok(cats) => {
            let mut levels: serde_json::Map<String, Value> = serde_json::Map::new();
            for c in cats {
                let skills = serde_json::to_value(&c.skills).unwrap_or(Value::Array(vec![]));
                let entry = levels
                    .entry(c.level)
                    .or_insert_with(|| Value::Array(vec![]));
                if let Value::Array(arr) = entry {
                    arr.push(json!({
                        "category": c.category,
                        "eval_type": c.eval_type,
                        "skills": skills,
                    }));
                }
            }
            Json(json!({ "levels": Value::Object(levels) })).into_response()
        }
        Err(e) => server_error("skills", e),
    }
}

/// All students' stored progress for a season. `?season_id=` required.
pub(crate) async fn get_progress(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let Some(sid) = q.get("season_id").and_then(|s| s.parse::<i64>().ok()) else {
        return e_error(StatusCode::BAD_REQUEST, "season_id required");
    };
    match with_db(&state, move |conn| {
        db::get_season_progress(conn.unwrap(), sid)
    })
    .await
    {
        Ok(rows) => {
            let data = serde_json::to_value(rows).unwrap_or(Value::Array(vec![]));
            Json(json!({ "progressData": data })).into_response()
        }
        Err(e) => server_error("progress", e),
    }
}

/// Persist one student's whole eval card for a season (replace-all). Validation
/// failures come back as 400.
pub(crate) async fn post_progress(State(state): State<SharedState>, body: Bytes) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let payload: db::SaveProgress = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return e_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    match with_db(&state, move |conn| {
        db::save_student_progress(conn.unwrap(), &payload)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(db::DbError::Value(m)) => e_error(StatusCode::BAD_REQUEST, m),
        Err(e) => server_error("save_progress", e),
    }
}
