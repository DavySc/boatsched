use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

pub(crate) async fn get_schedules(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let season_id = q.get("season_id").and_then(|s| s.parse::<i64>().ok());
    let (Some(_), Some(sid)) = (state.db.as_ref(), season_id) else {
        return Json(json!([])).into_response();
    };
    match with_db(&state, move |conn| db::list_schedules(conn.unwrap(), sid)).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("schedules", e),
    }
}

pub(crate) async fn get_schedule(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let id = match q.get("id").and_then(|s| s.parse::<i64>().ok()) {
        Some(id) => id,
        None => return e_error(StatusCode::BAD_REQUEST, "missing id"),
    };
    let result = with_db(&state, move |conn| {
        let conn = conn.unwrap();
        let rows = db::get_schedule(conn, id)?;
        let start = db::get_schedule_start_date(conn, id)?;
        Ok::<_, db::DbError>((rows, start))
    })
    .await;
    match result {
        Ok((rows, start_date)) => {
            Json(json!({ "rows": rows, "start_date": start_date })).into_response()
        }
        Err(e) => server_error("get_schedule", e),
    }
}

// ---------------------------------------------------------------------------
// Start-date / swap
// ---------------------------------------------------------------------------

pub(crate) async fn set_start_date(
    State(state): State<SharedState>,
    Path(schedule_id): Path<i64>,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let date_str = v
        .get("start_date")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    match with_db(&state, move |conn| {
        db::set_schedule_start_date(conn.unwrap(), schedule_id, date_str.as_deref())
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("set_start_date", e),
    }
}

pub(crate) async fn swap_endpoint(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    // Boat assignments are admin-only at all times — gate on the AdminCtx
    // capability before doing the mutation.
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => {
            return e_detail(
                StatusCode::FORBIDDEN,
                "Only admins may edit boat assignments",
            )
        }
    };
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (Some(schedule_id), Some(day), Some(name_a), Some(name_b)) = (
        v.get("schedule_id").and_then(json_int),
        v.get("day").and_then(|x| x.as_str()).map(str::to_string),
        v.get("name_a").and_then(|x| x.as_str()).map(str::to_string),
        v.get("name_b").and_then(|x| x.as_str()).map(str::to_string),
    ) else {
        return e_error(StatusCode::BAD_REQUEST, "'schedule_id'");
    };
    match with_db(&state, move |conn| {
        db::swap_assignment(conn.unwrap(), schedule_id, &day, &name_a, &name_b)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("swap", e),
    }
}

// ---------------------------------------------------------------------------
// Yjs commit + collab websocket
// ---------------------------------------------------------------------------

pub(crate) async fn commit_schedule(
    State(state): State<SharedState>,
    Path(schedule_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    // Admin-only at all times (admins are not blocked by the start-date lock).
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => {
            return e_detail(
                StatusCode::FORBIDDEN,
                "Only admins may edit boat assignments",
            )
        }
    };

    // Prefer the live in-memory doc (a session may be mid-edit); otherwise load
    // the persisted yjs_state. The doc is fully consumed before the next await.
    let outcome = {
        let doc = match state.collab.peek_doc(schedule_id) {
            Some(d) => d,
            None => {
                let st = with_db(&state, move |conn| {
                    db::get_yjs_state(conn.unwrap(), schedule_id).unwrap_or(None)
                })
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
            Json(json!({ "ok": false, "conflicts": arr })).into_response()
        }
        Ok(assignments) => {
            match with_db(&state, move |conn| {
                db::replace_assignments(conn.unwrap(), schedule_id, &assignments)
            })
            .await
            {
                Ok(()) => ok_true(),
                Err(e) => server_error("commit", e),
            }
        }
    }
}
