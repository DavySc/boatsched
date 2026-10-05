use crate::error::AppError;
use crate::*;
use serde_json::json;

pub(crate) async fn list_members_api(db: Db) -> Handled {
    let rows = db
        .run(db::list_members)
        .await
        .map_err(|e| AppError::internal("members", e))?;
    Ok(Json(rows).into_response())
}

pub(crate) async fn members_progression(db: Db) -> Handled {
    let out = db
        .run(|conn| {
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

            let mut mstmt =
                conn.prepare("SELECT id, name FROM member WHERE active=1 ORDER BY name")?;
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
    let out = out.map_err(|e| AppError::internal("progression", e))?;
    Ok(Json(out).into_response())
}

pub(crate) async fn rename_member_api(
    db: Db,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Handled {
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    let v = parse_json(&body)?;
    let name = v
        .get("name")
        .map(|n| json_str(n).trim().to_string())
        .ok_or_else(|| AppError::bad_request("'name'"))?;
    if name.is_empty() {
        return Err(AppError::bad_request("name must not be empty"));
    }
    match db
        .run(move |conn| db::rename_member(conn, member_id, &name))
        .await
    {
        Ok(()) => Ok(ok_true()),
        Err(db::DbError::Value(msg)) => Err(AppError::conflict(msg)),
        Err(e) => Err(AppError::internal("rename_member", e)),
    }
}

pub(crate) async fn delete_member_api(
    db: Db,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
) -> Handled {
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    match db.run(move |conn| db::delete_member(conn, member_id)).await {
        Ok(()) => Ok(ok_true()),
        Err(db::DbError::Value(msg)) => Err(AppError::conflict(msg)),
        Err(e) => Err(AppError::internal("delete_member", e)),
    }
}

pub(crate) async fn import_members_api(db: Db, headers: HeaderMap, body: Bytes) -> Handled {
    let state = db.state().clone();
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    too_large(&headers, state.opts.max_upload_bytes)?;
    let mut raw = body.to_vec();
    if raw.len() > state.opts.max_upload_bytes {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload too large",
        ));
    }
    let ctype = ctype_str(&headers);
    if ctype.contains("multipart") {
        raw = extract_multipart(&raw, &ctype);
    }
    let names = parse_names_csv(&String::from_utf8_lossy(&raw));
    if names.is_empty() {
        return Err(AppError::bad_request("no names found in upload"));
    }
    let result = db
        .run(move |conn| {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            db::seed_members(conn, &refs)
        })
        .await
        .map_err(|e| AppError::internal("import_members", e))?;
    Ok(Json(result).into_response())
}
