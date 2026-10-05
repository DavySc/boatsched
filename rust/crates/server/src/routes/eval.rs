use crate::error::AppError;
use crate::*;
use serde_json::json;

// Skill catalog + student progress (evaluation cards). All require a logged-in
// instructor (gated by `auth_middleware`); none require admin — recording an
// evaluation is an instructor activity.

/// The seeded skill catalog, grouped by level. Source of truth for the eval card.
pub(crate) async fn get_skills(db: Db) -> Handled {
    let cats = db
        .run(db::list_skill_catalog)
        .await
        .map_err(|e| AppError::internal("skills", e))?;
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
    Ok(Json(json!({ "levels": Value::Object(levels) })).into_response())
}

/// All students' stored progress for a season. `?season_id=` required.
pub(crate) async fn get_progress(db: Db, Query(q): Query<HashMap<String, String>>) -> Handled {
    let sid = q
        .get("season_id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| AppError::bad_request("season_id required"))?;
    let rows = db
        .run(move |conn| db::get_season_progress(conn, sid))
        .await
        .map_err(|e| AppError::internal("progress", e))?;
    let data = serde_json::to_value(rows).unwrap_or(Value::Array(vec![]));
    Ok(Json(json!({ "progressData": data })).into_response())
}

/// Persist one student's whole eval card for a season (replace-all). Validation
/// failures come back as 400.
pub(crate) async fn post_progress(db: Db, body: Bytes) -> Handled {
    let payload: db::SaveProgress =
        serde_json::from_slice(&body).map_err(|e| AppError::bad_request(e.to_string()))?;
    match db
        .run(move |conn| db::save_student_progress(conn, &payload))
        .await
    {
        Ok(()) => Ok(ok_true()),
        Err(db::DbError::Invalid(m)) => Err(AppError::bad_request(m)),
        Err(e) => Err(AppError::internal("save_progress", e)),
    }
}
