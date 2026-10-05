use crate::*;
use serde_json::json;

// ---------------------------------------------------------------------------
// Auth: login / logout / me
// ---------------------------------------------------------------------------

pub(crate) async fn api_login(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ip = client_ip(&headers);
    if state.auth.login_blocked(&ip) {
        return e_error(
            StatusCode::TOO_MANY_REQUESTS,
            "te veel pogingen, probeer later opnieuw",
        );
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(_) => return e_error(StatusCode::BAD_REQUEST, "invalid request"),
    };
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let password = v
        .get("password")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let auth = state.auth.clone();
    let open = with_db(&state, move |conn| auth.is_open(conn)).await;
    if state.db.is_none() || open {
        let token = state.auth.new_session(boatsched_auth::OPEN_MODE_MEMBER);
        let label = if name.is_empty() {
            "instructor".to_string()
        } else {
            name
        };
        let mut resp = Json(json!({ "ok": true, "name": label })).into_response();
        set_session_cookie(&mut resp, &token, &state.opts);
        return resp;
    }

    let (name_c, pw_c) = (name.clone(), password.clone());
    let member_id = with_db(&state, move |conn| {
        db::verify_instructor_login(conn.unwrap(), &name_c, &pw_c).unwrap_or(None)
    })
    .await;
    match member_id {
        None => {
            state.auth.record_login_failure(&ip);
            e_error(StatusCode::UNAUTHORIZED, "Onjuiste naam of wachtwoord.")
        }
        Some(mid) => {
            state.auth.clear_login_failures(&ip);
            let token = state.auth.new_session(mid);
            let mut resp = Json(json!({ "ok": true, "name": name })).into_response();
            set_session_cookie(&mut resp, &token, &state.opts);
            resp
        }
    }
}

pub(crate) async fn api_logout(headers: HeaderMap, State(state): State<SharedState>) -> Response {
    if let Some(token) = bsession(&headers) {
        state.auth.drop_session(&token);
    }
    let mut resp = Json(json!({ "ok": true })).into_response();
    if let Ok(hv) = HeaderValue::from_str("bsession=; Max-Age=0; Path=/") {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

pub(crate) async fn api_me(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let token = bsession(&headers);
    let member_id = auth_member(&state, token).await;
    let member_id = match member_id {
        Some(m) => m,
        None => return e_error(StatusCode::UNAUTHORIZED, "not authenticated"),
    };
    if member_id == boatsched_auth::OPEN_MODE_MEMBER || state.db.is_none() {
        return Json(json!({ "member_id": 0, "name": "instructor", "is_admin": true }))
            .into_response();
    }
    let auth = state.auth.clone();
    let (name, is_admin) = with_db(&state, move |conn| {
        let conn = conn.unwrap();
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT m.name, i.is_admin FROM member m \
                 JOIN instructor i ON i.member_id = m.id WHERE m.id=?",
                [member_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap_or(None);
        let is_admin = auth.is_open(Some(conn)) || row.as_ref().map_or(false, |(_, a)| *a != 0);
        (row.map(|(n, _)| n), is_admin)
    })
    .await;
    Json(json!({ "member_id": member_id, "name": name, "is_admin": is_admin })).into_response()
}

pub(crate) fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
