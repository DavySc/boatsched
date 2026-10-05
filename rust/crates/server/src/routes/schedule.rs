use crate::error::AppError;
use crate::*;
use serde_json::json;

const EDIT_DENIED: &str = "Only admins may edit boat assignments";

pub(crate) async fn get_schedules(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Handled {
    let season_id = q.get("season_id").and_then(|s| s.parse::<i64>().ok());
    let (Some(_), Some(sid)) = (state.db.as_ref(), season_id) else {
        return Ok(Json(json!([])).into_response());
    };
    let rows = with_db(&state, move |conn| db::list_schedules(conn.unwrap(), sid))
        .await
        .map_err(|e| AppError::internal("schedules", e))?;
    Ok(Json(rows).into_response())
}

pub(crate) async fn get_schedule(db: Db, Query(q): Query<HashMap<String, String>>) -> Handled {
    let id = q
        .get("id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| AppError::bad_request("missing id"))?;
    let (rows, start_date) = db
        .run(move |conn| {
            let rows = db::get_schedule(conn, id)?;
            let start = db::get_schedule_start_date(conn, id)?;
            Ok::<_, db::DbError>((rows, start))
        })
        .await
        .map_err(|e| AppError::internal("get_schedule", e))?;
    Ok(Json(json!({ "rows": rows, "start_date": start_date })).into_response())
}

pub(crate) async fn set_start_date(db: Db, Path(schedule_id): Path<i64>, body: Bytes) -> Handled {
    let v = parse_json(&body)?;
    let date_str = v
        .get("start_date")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    db.run(move |conn| db::set_schedule_start_date(conn, schedule_id, date_str.as_deref()))
        .await
        .map_err(|e| AppError::internal("set_start_date", e))?;
    Ok(ok_true())
}

pub(crate) async fn swap_endpoint(db: Db, headers: HeaderMap, body: Bytes) -> Handled {
    // Boat assignments are admin-only at all times — gate on the AdminCtx
    // capability before doing the mutation.
    let _admin = db
        .admin(
            &headers,
            AppError::detail(StatusCode::FORBIDDEN, EDIT_DENIED),
        )
        .await?;
    let v = parse_json(&body)?;
    let (Some(schedule_id), Some(day), Some(name_a), Some(name_b)) = (
        v.get("schedule_id").and_then(json_int),
        v.get("day").and_then(|x| x.as_str()).map(str::to_string),
        v.get("name_a").and_then(|x| x.as_str()).map(str::to_string),
        v.get("name_b").and_then(|x| x.as_str()).map(str::to_string),
    ) else {
        return Err(AppError::bad_request("'schedule_id'"));
    };
    db.run(move |conn| db::swap_assignment(conn, schedule_id, &day, &name_a, &name_b))
        .await
        .map_err(|e| AppError::internal("swap", e))?;
    Ok(ok_true())
}

pub(crate) async fn commit_schedule(
    db: Db,
    Path(schedule_id): Path<i64>,
    headers: HeaderMap,
) -> Handled {
    // Admin-only at all times (admins are not blocked by the start-date lock).
    let _admin = db
        .admin(
            &headers,
            AppError::detail(StatusCode::FORBIDDEN, EDIT_DENIED),
        )
        .await?;

    // Prefer the live in-memory doc (a session may be mid-edit); otherwise load
    // the persisted yjs_state. The doc is fully consumed before the next await.
    let outcome = {
        let doc = match db.state().collab.peek_doc(schedule_id) {
            Some(d) => d,
            None => {
                let st = db
                    .run(move |conn| db::get_yjs_state(conn, schedule_id).unwrap_or(None))
                    .await;
                let doc = yrs::Doc::new();
                {
                    use yrs::Transact;
                    let _ = doc.get_or_insert_map("schedule");
                    let _ = &doc.transact(); // ensure the map is registered
                }
                if let Some(bytes) = st {
                    collab::apply_update(&doc, &bytes);
                }
                doc
            }
        };
        collab::read_assignments(&doc)
    };

    match outcome {
        Err(conflicts) => {
            let arr: Vec<Value> = conflicts
                .into_iter()
                .map(|c| json!({ "name": c.name, "day": c.day, "boats": c.boats }))
                .collect();
            Ok(Json(json!({ "ok": false, "conflicts": arr })).into_response())
        }
        Ok(assignments) => {
            db.run(move |conn| db::replace_assignments(conn, schedule_id, &assignments))
                .await
                .map_err(|e| AppError::internal("commit", e))?;
            Ok(ok_true())
        }
    }
}
