use crate::error::AppError;
use crate::*;
use serde_json::json;

pub(crate) async fn get_instructors(State(state): State<SharedState>) -> Handled {
    if state.db.is_none() {
        return Ok(Json(json!([])).into_response());
    }
    let rows = with_db(&state, |conn| db::list_instructors(conn.unwrap()))
        .await
        .map_err(|e| AppError::internal("instructors", e))?;
    Ok(Json(rows).into_response())
}

/// `name` + `password` from a JSON body, name trimmed.
fn name_and_password(v: &Value) -> Result<(String, String), AppError> {
    match (v.get("name"), v.get("password")) {
        (Some(n), Some(p)) => Ok((json_str(n).trim().to_string(), json_str(p))),
        _ => Err(AppError::bad_request("'name'")),
    }
}

pub(crate) async fn create_instructor(db: Db, headers: HeaderMap, body: Bytes) -> Handled {
    db.admin(&headers, AppError::forbidden("admin required"))
        .await?;
    let v = parse_json(&body)?;
    let (name, password) = name_and_password(&v)?;
    if name.is_empty() || password.is_empty() {
        return Err(AppError::bad_request("name and password are required"));
    }
    weak_password(db.state(), &password)?;

    // Find or create the member, refuse if already an instructor, and make the
    // very first instructor an admin.
    let created = db
        .run(move |conn| {
            let member_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM member WHERE name=? COLLATE NOCASE",
                    [&name],
                    |r| r.get(0),
                )
                .optional()?;
            let member_id = match member_id {
                Some(id) => id,
                None => {
                    conn.execute("INSERT INTO member (name) VALUES (?)", [&name])?;
                    conn.last_insert_rowid()
                }
            };
            let already: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM instructor WHERE member_id=?",
                    [member_id],
                    |r| r.get(0),
                )
                .optional()?;
            if already.is_some() {
                return Ok::<Option<i64>, rusqlite::Error>(None);
            }
            let no_admins: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM instructor WHERE is_admin=1 LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            conn.execute(
                "INSERT INTO instructor (member_id, is_admin) VALUES (?, ?)",
                rusqlite::params![member_id, no_admins.is_none() as i64],
            )?;
            Ok(Some(member_id))
        })
        .await
        .map_err(|e| AppError::internal("create_instructor", e))?;
    let member_id = created.ok_or_else(|| AppError::conflict("already an instructor"))?;

    db.run(move |conn| db::set_instructor_password(conn, member_id, &password))
        .await
        .map_err(|e| AppError::internal("create_instructor/password", e))?;
    Ok(ok_true())
}

pub(crate) async fn set_password(db: Db, headers: HeaderMap, body: Bytes) -> Handled {
    let state = db.state().clone();
    let v = parse_json(&body)?;
    let (name, password) = name_and_password(&v)?;
    if password.is_empty() {
        return Err(AppError::bad_request("password must not be empty"));
    }
    weak_password(&state, &password)?;
    let target: i64 = db
        .run(move |conn| {
            conn.query_row(
                "SELECT i.member_id FROM instructor i JOIN member m ON m.id = i.member_id \
                 WHERE m.name=? COLLATE NOCASE",
                [&name],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
        })
        .await
        .ok_or_else(|| AppError::not_found("not an instructor"))?;

    let token = bsession(&headers);
    let auth = state.auth.clone();
    let open = with_db(&state, move |conn| auth.is_open(conn)).await;
    if !open {
        let caller = auth_member(&state, token.clone())
            .await
            .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "not authenticated"))?;
        if caller != target && !current_is_admin(&state, token).await {
            return Err(AppError::forbidden(
                "admin required to change another instructor's password",
            ));
        }
    }
    db.run(move |conn| db::set_instructor_password(conn, target, &password))
        .await
        .map_err(|e| AppError::internal("set_password", e))?;
    Ok(ok_true())
}

fn weak_password(state: &SharedState, password: &str) -> Result<(), AppError> {
    if password.chars().count() < state.opts.min_password_len {
        return Err(AppError::bad_request(format!(
            "wachtwoord moet minstens {} tekens zijn",
            state.opts.min_password_len
        )));
    }
    Ok(())
}

pub(crate) async fn set_instructor_admin(
    db: Db,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Handled {
    let admin = db
        .admin(&headers, AppError::forbidden("admin required"))
        .await?;
    let v = parse_json(&body)?;
    let is_admin = v
        .get("is_admin")
        .map(json_bool)
        .ok_or_else(|| AppError::bad_request("'is_admin'"))?;
    // Lockout protection: an admin cannot revoke their own admin.
    if guard_set_admin(&admin, member_id, is_admin).is_err() {
        return Err(AppError::bad_request("cannot remove own admin status"));
    }
    let exists = db
        .run(move |conn| {
            conn.query_row(
                "SELECT 1 FROM instructor WHERE member_id=?",
                [member_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .unwrap_or(None)
            .is_some()
        })
        .await;
    if !exists {
        return Err(AppError::not_found("not an instructor"));
    }
    db.run(move |conn| db::set_admin(conn, member_id, is_admin))
        .await
        .map_err(|e| AppError::internal("set_admin", e))?;
    Ok(ok_true())
}
