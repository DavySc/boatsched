use crate::error::AppError;
use crate::*;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde_json::json;

pub(crate) fn ok_true() -> Response {
    Json(json!({ "ok": true })).into_response()
}

/// Run `f` with the (optional) DB connection on a blocking thread over the shared
/// single connection.
pub(crate) async fn with_db<T, F>(state: &SharedState, f: F) -> T
where
    F: FnOnce(Option<&Connection>) -> T + Send + 'static,
    T: Send + 'static,
{
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let guard = db
            .as_ref()
            .map(|m| m.lock().unwrap_or_else(|p| p.into_inner()));
        f(guard.as_deref())
    })
    .await
    .expect("db task panicked")
}

/// Proof that the server has a database: extracting it answers 503 otherwise, so
/// handlers that need the DB don't repeat the check or unwrap the connection.
pub(crate) struct Db(SharedState);

#[axum::async_trait]
impl FromRequestParts<SharedState> for Db {
    type Rejection = AppError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        if state.db.is_some() {
            Ok(Db(state.clone()))
        } else {
            Err(AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no database",
            ))
        }
    }
}

impl Db {
    pub(crate) fn state(&self) -> &SharedState {
        &self.0
    }

    pub(crate) async fn run<T, F>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T + Send + 'static,
        T: Send + 'static,
    {
        with_db(&self.0, move |conn| {
            f(conn.expect("Db extractor guarantees a connection"))
        })
        .await
    }

    /// The [`AdminCtx`] capability for the request, or `denied` if the caller
    /// isn't an admin. Admin-only handlers gate the mutation on `Ok`.
    pub(crate) async fn admin(
        &self,
        headers: &HeaderMap,
        denied: AppError,
    ) -> Result<AdminCtx, AppError> {
        require_admin(&self.0, bsession(headers))
            .await
            .map_err(|_| denied)
    }
}

pub(crate) async fn auth_member(state: &SharedState, token: Option<String>) -> Option<i64> {
    let auth = state.auth.clone();
    with_db(state, move |conn| auth.auth_member(token.as_deref(), conn)).await
}

/// Mirrors `_current_is_admin`: resolve the session and ask if it's an admin
/// (always true in open mode).
pub(crate) async fn current_is_admin(state: &SharedState, token: Option<String>) -> bool {
    let auth = state.auth.clone();
    with_db(state, move |conn| {
        let mid = auth.auth_member(token.as_deref(), conn);
        auth.member_is_admin(mid, conn)
    })
    .await
}

/// Obtain the [`AdminCtx`] capability for the request, or the reason it can't be
/// had. Admin-only handlers call this and gate the DB mutation on `Ok`.
pub(crate) async fn require_admin(
    state: &SharedState,
    token: Option<String>,
) -> Result<AdminCtx, AuthError> {
    let auth = state.auth.clone();
    with_db(state, move |conn| {
        auth.require_admin(token.as_deref(), conn)
    })
    .await
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

pub(crate) fn bsession(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, "bsession")
}

pub(crate) fn set_session_cookie(resp: &mut Response, token: &str, opts: &ServerOptions) {
    let secs = opts.session_ttl.as_secs();
    let mut v = format!("bsession={token}; HttpOnly; SameSite=Lax; Max-Age={secs}; Path=/");
    if opts.secure_cookies {
        v.push_str("; Secure");
    }
    if let Ok(hv) = HeaderValue::from_str(&v) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
}

/// Cheap content-length guard, mirroring `_too_large`: declared size over the
/// cap → 413; unparseable → 400.
pub(crate) fn too_large(headers: &HeaderMap, max: usize) -> Result<(), AppError> {
    let Some(cl) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(());
    };
    match cl.parse::<usize>() {
        Ok(n) if n > max => Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload too large",
        )),
        Ok(_) => Ok(()),
        Err(_) => Err(AppError::bad_request("invalid content-length")),
    }
}

/// Parse a JSON body into a `Value`, or a 400 response. Mirrors the Python
/// `body = await request.json()` + `except Exception: 400` pattern (the exact
/// message text for malformed JSON differs from Python — documented divergence).
pub(crate) fn parse_json(body: &Bytes) -> Result<Value, AppError> {
    serde_json::from_slice(body).map_err(|e| AppError::bad_request(e.to_string()))
}

pub(crate) fn ctype_str(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Auth middleware (mirrors _auth_middleware): gate non-public paths, then add
// defence-in-depth headers.
// ---------------------------------------------------------------------------

pub(crate) const PUBLIC_PATHS: [&str; 5] = [
    "/",
    "/index.html",
    "/api/auth/login",
    "/api/auth/logout",
    "/api/health",
];

pub(crate) async fn auth_middleware(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    // WebSocket upgrades aren't gated here (Python's HTTP middleware doesn't run
    // for the WS scope); /collab does its own auth with close codes.
    if !PUBLIC_PATHS.contains(&path.as_str()) && !path.starts_with("/collab/") {
        let token = bsession(req.headers());
        if auth_member(&state, token).await.is_none() {
            return AppError::new(StatusCode::UNAUTHORIZED, "not authenticated").into_response();
        }
    }
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    for (name, val) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        if !h.contains_key(name) {
            h.insert(
                header::HeaderName::from_static(name),
                HeaderValue::from_static(val),
            );
        }
    }
    resp
}
