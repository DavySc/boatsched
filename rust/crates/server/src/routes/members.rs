use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Members
// ---------------------------------------------------------------------------

pub(crate) async fn list_members_api(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    match with_db(&state, |conn| db::list_members(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("members", e),
    }
}

pub(crate) async fn members_progression(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let out = with_db(&state, |conn| {
        let conn = conn.unwrap();
        // instructor ids (excluded)
        let mut stmt = conn.prepare("SELECT member_id FROM instructor")?;
        let instructor_ids: std::collections::HashSet<i64> = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<_>>()?;

        let mut sstmt = conn.prepare("SELECT id, year FROM season ORDER BY year")?;
        let seasons: Vec<Value> = sstmt
            .query_map([], |r| {
                Ok(json!({ "id": r.get::<_, i64>(0)?, "year": r.get::<_, i64>(1)? }))
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut estmt = conn.prepare("SELECT member_id, season_id, level FROM enrollment")?;
        let mut by_member: HashMap<i64, HashMap<i64, String>> = HashMap::new();
        let rows = estmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (mid, sid, lvl) = row?;
            by_member.entry(mid).or_default().insert(sid, lvl);
        }

        let mut mstmt = conn.prepare("SELECT id, name FROM member WHERE active=1 ORDER BY name")?;
        let members: Vec<Value> = mstmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(id, _)| !instructor_ids.contains(id))
            .map(|(id, name)| {
                let levels: serde_json::Map<String, Value> = by_member
                    .get(&id)
                    .map(|m| {
                        m.iter()
                            .map(|(sid, lv)| (sid.to_string(), json!(lv)))
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "id": id, "name": name, "levels": Value::Object(levels) })
            })
            .collect();

        Ok::<Value, rusqlite::Error>(json!({ "seasons": seasons, "members": members }))
    })
    .await;
    match out {
        Ok(v) => Json(v).into_response(),
        Err(e) => server_error("progression", e),
    }
}

pub(crate) async fn rename_member_api(
    State(state): State<SharedState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    if !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let name = match v.get("name") {
        Some(n) => json_str(n).trim().to_string(),
        None => return e_error(StatusCode::BAD_REQUEST, "'name'"),
    };
    if name.is_empty() {
        return e_error(StatusCode::BAD_REQUEST, "name must not be empty");
    }
    match with_db(&state, move |conn| {
        db::rename_member(conn.unwrap(), member_id, &name)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(db::DbError::Value(msg)) => e_error(StatusCode::CONFLICT, msg),
        Err(e) => server_error("rename_member", e),
    }
}

pub(crate) async fn delete_member_api(
    State(state): State<SharedState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    if !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    match with_db(&state, move |conn| {
        db::delete_member(conn.unwrap(), member_id)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(db::DbError::Value(msg)) => e_error(StatusCode::CONFLICT, msg),
        Err(e) => server_error("delete_member", e),
    }
}

pub(crate) async fn import_members_api(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    if !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    if let Some(r) = too_large(&headers, state.opts.max_upload_bytes) {
        return r;
    }
    let mut raw = body.to_vec();
    if raw.len() > state.opts.max_upload_bytes {
        return e_error(StatusCode::PAYLOAD_TOO_LARGE, "payload too large");
    }
    let ctype = ctype_str(&headers);
    if ctype.contains("multipart") {
        raw = extract_multipart(&raw, &ctype);
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let names = parse_names_csv(&text);
    if names.is_empty() {
        return e_error(StatusCode::BAD_REQUEST, "no names found in upload");
    }
    let name_refs: Vec<String> = names;
    match with_db(&state, move |conn| {
        let refs: Vec<&str> = name_refs.iter().map(String::as_str).collect();
        db::seed_members(conn.unwrap(), &refs)
    })
    .await
    {
        Ok(result) => Json(result).into_response(),
        Err(e) => server_error("import_members", e),
    }
}
