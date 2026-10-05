use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Instructors / password management
// ---------------------------------------------------------------------------

pub(crate) async fn get_instructors(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_instructors(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("instructors", e),
    }
}

pub(crate) async fn create_instructor(
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
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (name, password) = match (v.get("name"), v.get("password")) {
        (Some(n), Some(p)) => (json_str(n).trim().to_string(), json_str(p)),
        _ => return e_error(StatusCode::BAD_REQUEST, "'name'"),
    };
    if name.is_empty() || password.is_empty() {
        return e_error(StatusCode::BAD_REQUEST, "name and password are required");
    }
    if let Some(r) = weak_password(&state, &password) {
        return r;
    }

    // Replicates the Python `_create` closure: find/create the member, refuse if
    // already an instructor, auto-admin the first instructor.
    let name_c = name.clone();
    let created = with_db(&state, move |conn| {
        let conn = conn.unwrap();
        let member_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM member WHERE name=? COLLATE NOCASE",
                [&name_c],
                |r| r.get(0),
            )
            .optional()?;
        let member_id = match member_id {
            Some(id) => id,
            None => {
                conn.execute("INSERT INTO member (name) VALUES (?)", [&name_c])?;
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
    .await;

    let member_id = match created {
        Ok(Some(id)) => id,
        Ok(None) => return e_error(StatusCode::CONFLICT, "already an instructor"),
        Err(e) => return server_error("create_instructor", e),
    };
    match with_db(&state, move |conn| {
        db::set_instructor_password(conn.unwrap(), member_id, &password)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("create_instructor/password", e),
    }
}

pub(crate) async fn set_password(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (name, password) = match (v.get("name"), v.get("password")) {
        (Some(n), Some(p)) => (json_str(n).trim().to_string(), json_str(p)),
        _ => return e_error(StatusCode::BAD_REQUEST, "'name'"),
    };
    if password.is_empty() {
        return e_error(StatusCode::BAD_REQUEST, "password must not be empty");
    }
    if let Some(r) = weak_password(&state, &password) {
        return r;
    }
    let name_c = name.clone();
    let target: Option<i64> = with_db(&state, move |conn| {
        conn.unwrap()
            .query_row(
                "SELECT i.member_id FROM instructor i JOIN member m ON m.id = i.member_id \
                 WHERE m.name=? COLLATE NOCASE",
                [&name_c],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
    })
    .await;
    let target = match target {
        Some(id) => id,
        None => return e_error(StatusCode::NOT_FOUND, "not an instructor"),
    };

    let token = bsession(&headers);
    let auth = state.auth.clone();
    let token_c = token.clone();
    let open = with_db(&state, move |conn| auth.is_open(conn)).await;
    if !open {
        let caller = auth_member(&state, token_c).await;
        let caller = match caller {
            Some(c) => c,
            None => return e_error(StatusCode::UNAUTHORIZED, "not authenticated"),
        };
        if caller != target && !current_is_admin(&state, token).await {
            return e_error(
                StatusCode::FORBIDDEN,
                "admin required to change another instructor's password",
            );
        }
    }
    match with_db(&state, move |conn| {
        db::set_instructor_password(conn.unwrap(), target, &password)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("set_password", e),
    }
}

pub(crate) fn weak_password(state: &SharedState, password: &str) -> Option<Response> {
    if password.chars().count() < state.opts.min_password_len {
        return Some(e_error(
            StatusCode::BAD_REQUEST,
            format!(
                "wachtwoord moet minstens {} tekens zijn",
                state.opts.min_password_len
            ),
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// Set-admin
// ---------------------------------------------------------------------------

pub(crate) async fn set_instructor_admin(
    State(state): State<SharedState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => return e_error(StatusCode::FORBIDDEN, "admin required"),
    };
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let is_admin = match v.get("is_admin") {
        Some(x) => json_bool(x),
        None => return e_error(StatusCode::BAD_REQUEST, "'is_admin'"),
    };
    // Lockout protection: an admin cannot revoke their own admin (uses the
    // capability's acting member — the typed equivalent of comparing
    // _auth_member(token) == member_id).
    if guard_set_admin(&admin, member_id, is_admin).is_err() {
        return e_error(StatusCode::BAD_REQUEST, "cannot remove own admin status");
    }
    let exists = with_db(&state, move |conn| {
        conn.unwrap()
            .query_row(
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
        return e_error(StatusCode::NOT_FOUND, "not an instructor");
    }
    match with_db(&state, move |conn| {
        db::set_admin(conn.unwrap(), member_id, is_admin)
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("set_admin", e),
    }
}
