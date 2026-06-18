//! axum HTTP server — a route-for-route port of `server.py`'s `create_app`.
//!
//! Scope of stage (d): every JSON/CSV HTTP route. The yrs-coupled endpoints
//! (`POST /schedule/{id}/commit` and the `/collab/{id}` websocket) are
//! deferred to stage (f) where the CRDT layer lands; they are intentionally not
//! registered here yet.
//!
//! Parity notes:
//! - Status codes, response key names (`error` vs `detail`), and the (Dutch)
//!   message strings match the Python responses.
//! - Authorization goes through [`boatsched_auth`]: admin-only handlers obtain
//!   an [`AdminCtx`] capability before touching the DB mutation, so the
//!   privileged call is unreachable without it (the type-level encoding from
//!   stage c).
//! - DB work runs on `spawn_blocking` over a single `Arc<Mutex<Connection>>`,
//!   mirroring the Python single-connection + `asyncio.to_thread` model.

pub mod collab;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use boatsched_auth::{guard_set_admin, AdminCtx, AuthConfig, AuthError, Authenticator};
use boatsched_core::Config;
use boatsched_db as db;
use boatsched_roster::{parse_roster_bytes, RosterError};
use boatsched_solver::solve;
use rusqlite::{Connection, OptionalExtension};

/// The embedded SPA, identical to the asset the Python server serves at `/`.
const VIEWER_HTML: &str = include_str!("../../../../src/boatsched/viewer.html");

/// Valid student levels, matching `server.py`'s `VALID_LEVELS`.
const VALID_LEVELS: [&str; 4] = ["k", "b", "A", "B"];

const JOB_TTL: Duration = Duration::from_secs(3600);
const JOB_MAX: usize = 200;

/// Security/operational knobs, mirroring the `create_app` keyword arguments.
/// Defaults are the permissive dev values (so tests behave like the Python
/// suite); `main` would flip them to strict production values.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub require_auth: bool,
    pub secure_cookies: bool,
    pub session_ttl: Duration,
    pub login_max_failures: usize,
    pub login_window: Duration,
    pub max_upload_bytes: usize,
    pub max_ws_message_bytes: usize,
    pub min_password_len: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            require_auth: false,
            secure_cookies: false,
            session_ttl: Duration::from_secs(12 * 3600),
            login_max_failures: 20,
            login_window: Duration::from_secs(300),
            max_upload_bytes: 5 * 1024 * 1024,
            max_ws_message_bytes: 1024 * 1024,
            min_password_len: 1,
        }
    }
}

#[derive(Clone)]
enum JobStatus {
    Running,
    Done,
    Error,
}

struct Job {
    status: JobStatus,
    result: Option<Vec<boatsched_core::ScheduleRow>>,
    stats: Option<boatsched_solver::Stats>,
    error: Option<String>,
    schedule_id: Option<i64>,
    created: Instant,
}

/// Shared application state. Cloneable handle is `SharedState`.
pub struct AppState {
    cfg: Config,
    db: Option<Arc<Mutex<Connection>>>,
    auth: Arc<Authenticator>,
    jobs: Mutex<HashMap<String, Job>>,
    collab: collab::CollabState,
    opts: ServerOptions,
}

pub type SharedState = Arc<AppState>;

/// Build the router, mirroring `server.py:create_app`. `db = None` is dev/no-DB
/// mode (open mode unless `require_auth`).
pub fn create_app(cfg: Config, db_conn: Option<Connection>, opts: ServerOptions) -> Router {
    // Run the idempotent migrations on the connection, exactly as the Python
    // create_app does before serving (no-ops on a fresh DB).
    if let Some(conn) = &db_conn {
        db::schema::run_migrations(conn).expect("database migrations failed");
    }
    let auth = Authenticator::new(AuthConfig {
        require_auth: opts.require_auth,
        session_ttl: opts.session_ttl,
        login_max_failures: opts.login_max_failures,
        login_window: opts.login_window,
    });
    let state: SharedState = Arc::new(AppState {
        cfg,
        db: db_conn.map(|c| Arc::new(Mutex::new(c))),
        auth: Arc::new(auth),
        jobs: Mutex::new(HashMap::new()),
        collab: collab::CollabState::new(),
        opts,
    });

    Router::new()
        .route("/", get(root))
        .route("/index.html", get(root))
        .route("/config", get(config_endpoint))
        .route("/instructors", get(get_instructors))
        .route("/instructor", post(create_instructor))
        .route("/set-password", post(set_password))
        .route("/seasons", get(get_seasons).post(create_season))
        .route("/boats", get(get_boats))
        .route("/boat", post(rename_boat))
        .route("/schedules", get(get_schedules))
        .route("/schedule", get(get_schedule))
        .route("/solve", post(solve_endpoint))
        .route("/status", get(get_status))
        .route("/schedule/:id/start-date", post(set_start_date))
        .route("/schedule/:id/commit", post(commit_schedule))
        .route("/collab/:id", get(collab_ws))
        .route("/swap", post(swap_endpoint))
        .route("/api/auth/login", post(api_login))
        .route("/api/auth/logout", post(api_logout))
        .route("/api/auth/me", get(api_me))
        .route("/api/members", get(list_members_api))
        .route("/api/members/progression", get(members_progression))
        .route("/api/members/import", post(import_members_api))
        .route("/api/members/:id", put(rename_member_api).delete(delete_member_api))
        .route("/api/instructors/:id/set-admin", post(set_instructor_admin))
        .route(
            "/enrollment/:eid/member/:mid/level",
            patch(set_student_level),
        )
        .route("/enrollment/:eid/students/export", get(export_students))
        .route("/api/health", get(api_health))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Response + request helpers
// ---------------------------------------------------------------------------

fn e_error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn e_detail(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": msg.into() }))).into_response()
}

/// Generic 500, mirroring `_server_error`: log the real cause, return an opaque
/// body so DB messages / paths don't leak.
fn server_error(context: &str, e: impl std::fmt::Debug) -> Response {
    eprintln!("[error] {context}: {e:?}");
    e_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

fn ok_true() -> Response {
    Json(json!({ "ok": true })).into_response()
}

/// Run `f` with the (optional) DB connection on a blocking thread, mirroring
/// `asyncio.to_thread` over the shared single connection.
async fn with_db<T, F>(state: &SharedState, f: F) -> T
where
    F: FnOnce(Option<&Connection>) -> T + Send + 'static,
    T: Send + 'static,
{
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let guard = db.as_ref().map(|m| m.lock().unwrap());
        f(guard.as_deref())
    })
    .await
    .expect("db task panicked")
}

async fn auth_member(state: &SharedState, token: Option<String>) -> Option<i64> {
    let auth = state.auth.clone();
    with_db(state, move |conn| auth.auth_member(token.as_deref(), conn)).await
}

/// Mirrors `_current_is_admin`: resolve the session and ask if it's an admin
/// (always true in open mode).
async fn current_is_admin(state: &SharedState, token: Option<String>) -> bool {
    let auth = state.auth.clone();
    with_db(state, move |conn| {
        let mid = auth.auth_member(token.as_deref(), conn);
        auth.member_is_admin(mid, conn)
    })
    .await
}

/// Obtain the [`AdminCtx`] capability for the request, or the reason it can't be
/// had. Admin-only handlers call this and gate the DB mutation on `Ok`.
async fn require_admin(state: &SharedState, token: Option<String>) -> Result<AdminCtx, AuthError> {
    let auth = state.auth.clone();
    with_db(state, move |conn| auth.require_admin(token.as_deref(), conn)).await
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

fn bsession(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, "bsession")
}

fn set_session_cookie(resp: &mut Response, token: &str, opts: &ServerOptions) {
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
fn too_large(headers: &HeaderMap, max: usize) -> Option<Response> {
    let cl = headers.get(header::CONTENT_LENGTH)?.to_str().ok()?;
    match cl.parse::<usize>() {
        Ok(n) if n > max => Some(e_error(StatusCode::PAYLOAD_TOO_LARGE, "payload too large")),
        Ok(_) => None,
        Err(_) => Some(e_error(StatusCode::BAD_REQUEST, "invalid content-length")),
    }
}

/// Parse a JSON body into a `Value`, or a 400 response. Mirrors the Python
/// `body = await request.json()` + `except Exception: 400` pattern (the exact
/// message text for malformed JSON differs from Python — documented divergence).
fn parse_json(body: &Bytes) -> Result<Value, Response> {
    serde_json::from_slice(body).map_err(|e| e_error(StatusCode::BAD_REQUEST, e.to_string()))
}

fn ctype_str(headers: &HeaderMap) -> String {
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

const PUBLIC_PATHS: [&str; 5] = [
    "/",
    "/index.html",
    "/api/auth/login",
    "/api/auth/logout",
    "/api/health",
];

async fn auth_middleware(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    // WebSocket upgrades aren't gated here (Python's HTTP middleware doesn't run
    // for the WS scope); /collab does its own auth with close codes.
    if !PUBLIC_PATHS.contains(&path.as_str()) && !path.starts_with("/collab/") {
        let token = bsession(req.headers());
        if auth_member(&state, token).await.is_none() {
            return e_error(StatusCode::UNAUTHORIZED, "not authenticated");
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

// ---------------------------------------------------------------------------
// Static + config
// ---------------------------------------------------------------------------

async fn root() -> Html<&'static str> {
    Html(VIEWER_HTML)
}

async fn config_endpoint(State(state): State<SharedState>) -> Response {
    Json(json!({
        "boats": state.cfg.boats,
        "levels": state.cfg.levels,
        "restricted": state.cfg.restricted,
        "diverse": state.cfg.diverse,
        "db": state.db.is_some(),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Instructors / password management
// ---------------------------------------------------------------------------

async fn get_instructors(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_instructors(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("instructors", e),
    }
}

async fn create_instructor(
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
            .query_row("SELECT 1 FROM instructor WHERE is_admin=1 LIMIT 1", [], |r| {
                r.get(0)
            })
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

async fn set_password(
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

fn weak_password(state: &SharedState, password: &str) -> Option<Response> {
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
// Seasons
// ---------------------------------------------------------------------------

async fn get_seasons(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_seasons(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("seasons", e),
    }
}

async fn create_season(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database configured");
    }
    if !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let year = match v.get("year").and_then(json_int) {
        Some(y) => y,
        None => return e_error(StatusCode::BAD_REQUEST, "'year'"),
    };
    match with_db(&state, move |conn| db::create_season(conn.unwrap(), year)).await {
        Ok(sid) => Json(json!({ "id": sid, "year": year })).into_response(),
        Err(e) => server_error("create_season", e),
    }
}

// ---------------------------------------------------------------------------
// Boats
// ---------------------------------------------------------------------------

async fn get_boats(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return Json(json!([])).into_response();
    }
    match with_db(&state, |conn| db::list_boats(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("boats", e),
    }
}

async fn rename_boat(State(state): State<SharedState>, body: Bytes) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let boat_id = match v.get("id").and_then(json_int) {
        Some(b) => b,
        None => return e_error(StatusCode::BAD_REQUEST, "'id'"),
    };
    // `body.get("name") or None` → empty/missing becomes NULL.
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    match with_db(&state, move |conn| {
        db::rename_boat(conn.unwrap(), boat_id, name.as_deref())
    })
    .await
    {
        Ok(()) => ok_true(),
        Err(e) => server_error("rename_boat", e),
    }
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

async fn get_schedules(
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

async fn get_schedule(
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
// Solve / status
// ---------------------------------------------------------------------------

async fn solve_endpoint(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if state.db.is_some() && !current_is_admin(&state, bsession(&headers)).await {
        return e_error(StatusCode::FORBIDDEN, "admin required");
    }
    if let Some(r) = too_large(&headers, state.opts.max_upload_bytes) {
        return r;
    }
    let ctype = ctype_str(&headers);
    let mut raw = body.to_vec();
    if raw.len() > state.opts.max_upload_bytes {
        return e_error(StatusCode::PAYLOAD_TOO_LARGE, "payload too large");
    }
    if ctype.starts_with("multipart/form-data") {
        raw = extract_multipart(&raw, &ctype);
    }
    let kids = match parse_roster_bytes(&raw) {
        Ok(k) => k,
        Err(RosterError(msg)) => return e_error(StatusCode::BAD_REQUEST, msg),
    };
    let season_id = q.get("season_id").cloned();

    if state.db.is_some() {
        let sid = season_id.as_ref().and_then(|s| s.parse::<i64>().ok());
        let kids_c = kids.clone();
        let res = with_db(&state, move |conn| {
            db::add_or_update_members(conn.unwrap(), &kids_c, sid)
        })
        .await;
        match res {
            Ok(()) => {}
            Err(db::DbError::Runtime(msg)) => return e_error(StatusCode::BAD_REQUEST, msg),
            Err(e) => return server_error("solve/add_members", e),
        }
    }

    let jid = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    {
        let mut jobs = state.jobs.lock().unwrap();
        prune_jobs(&mut jobs);
        jobs.insert(
            jid.clone(),
            Job {
                status: JobStatus::Running,
                result: None,
                stats: None,
                error: None,
                schedule_id: None,
                created: Instant::now(),
            },
        );
    }
    let state2 = state.clone();
    let jid2 = jid.clone();
    let cfg = state.cfg.clone();
    tokio::spawn(async move {
        run_job(state2, jid2, kids, cfg, season_id).await;
    });
    Json(json!({ "id": jid })).into_response()
}

async fn run_job(
    state: SharedState,
    jid: String,
    kids: Vec<boatsched_core::Kid>,
    cfg: Config,
    season_id: Option<String>,
) {
    let cfg_solve = cfg.clone();
    let solved =
        tokio::task::spawn_blocking(move || solve(&kids, &cfg_solve, None)).await;

    match solved {
        Ok(Ok((rows, stats))) => {
            let mut schedule_id = None;
            if let (Some(_), Some(sid)) = (
                state.db.as_ref(),
                season_id.as_ref().and_then(|s| s.parse::<i64>().ok()),
            ) {
                let cfg_json = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());
                let rows_c = rows.clone();
                // Initialise the Yjs doc from the result and persist it as
                // yjs_state, exactly like Python's _run_job.
                let yjs = collab::init_doc_update_from_rows(&rows);
                let saved = with_db(&state, move |conn| {
                    let conn = conn.unwrap();
                    let id = db::save_schedule(conn, sid, &cfg_json, &rows_c)?;
                    db::set_yjs_state(conn, id, &yjs)?;
                    Ok::<i64, db::DbError>(id)
                })
                .await;
                match saved {
                    Ok(id) => schedule_id = Some(id),
                    Err(e) => eprintln!("[db] save_schedule failed: {e:?}"),
                }
            }
            let mut jobs = state.jobs.lock().unwrap();
            if let Some(job) = jobs.get_mut(&jid) {
                job.status = JobStatus::Done;
                job.result = Some(rows);
                job.stats = Some(stats);
                job.schedule_id = schedule_id;
            }
        }
        Ok(Err(e)) => set_job_error(&state, &jid, e.to_string()),
        Err(e) => set_job_error(&state, &jid, format!("solver task panicked: {e}")),
    }
}

fn set_job_error(state: &SharedState, jid: &str, msg: String) {
    let mut jobs = state.jobs.lock().unwrap();
    if let Some(job) = jobs.get_mut(jid) {
        job.status = JobStatus::Error;
        job.error = Some(if msg.is_empty() { "error".into() } else { msg });
    }
}

fn prune_jobs(jobs: &mut HashMap<String, Job>) {
    let now = Instant::now();
    jobs.retain(|_, j| {
        matches!(j.status, JobStatus::Running) || now.duration_since(j.created) <= JOB_TTL
    });
    if jobs.len() > JOB_MAX {
        let mut by_age: Vec<(String, Instant)> =
            jobs.iter().map(|(k, v)| (k.clone(), v.created)).collect();
        by_age.sort_by_key(|(_, t)| *t);
        for (k, _) in by_age.into_iter().take(jobs.len() - JOB_MAX) {
            jobs.remove(&k);
        }
    }
}

async fn get_status(
    State(state): State<SharedState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let id = q.get("id").cloned().unwrap_or_default();
    let jobs = state.jobs.lock().unwrap();
    let job = match jobs.get(&id) {
        Some(j) => j,
        None => return e_error(StatusCode::NOT_FOUND, "no such job"),
    };
    let mut out = serde_json::Map::new();
    match job.status {
        JobStatus::Running => {
            out.insert("status".into(), json!("running"));
        }
        JobStatus::Done => {
            out.insert("status".into(), json!("done"));
            out.insert("result".into(), serde_json::to_value(&job.result).unwrap());
            out.insert("stats".into(), stats_json(job.stats.as_ref()));
            if let Some(sid) = job.schedule_id {
                out.insert("schedule_id".into(), json!(sid));
            }
        }
        JobStatus::Error => {
            out.insert("status".into(), json!("error"));
            out.insert("error".into(), json!(job.error));
        }
    }
    Json(Value::Object(out)).into_response()
}

fn stats_json(stats: Option<&boatsched_solver::Stats>) -> Value {
    match stats {
        Some(s) => json!({
            "mixed": s.mixed,
            "distinct": s.distinct,
            "repeats": s.repeats,
            "status_C": s.status_c,
        }),
        None => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// Start-date / swap
// ---------------------------------------------------------------------------

async fn set_start_date(
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

async fn swap_endpoint(
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
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may edit boat assignments"),
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

async fn commit_schedule(
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
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may edit boat assignments"),
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

async fn collab_ws(
    State(state): State<SharedState>,
    Path(schedule_id): Path<i64>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // The HTTP auth middleware is skipped for /collab; do the checks here. We
    // deny the handshake (HTTP error) rather than accepting then closing — the
    // connection is refused either way (documented divergence from the Python
    // post-accept ws.close(1011/1008)).
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let token = bsession(&headers);
    if auth_member(&state, token.clone()).await.is_none() {
        return e_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    // Admins edit; everyone else observes (their update frames are dropped).
    let is_admin = current_is_admin(&state, token).await;
    ws.on_upgrade(move |socket| handle_collab(state, schedule_id, is_admin, socket))
}

async fn handle_collab(
    state: SharedState,
    schedule_id: i64,
    is_admin: bool,
    mut socket: WebSocket,
) {
    use tokio::sync::broadcast::error::RecvError;

    // Lazily load persisted state, then join the room.
    let initial = with_db(&state, move |conn| {
        conn.and_then(|c| db::get_yjs_state(c, schedule_id).unwrap_or(None))
    })
    .await;
    let joined = state.collab.join(schedule_id, initial.as_deref());
    let conn_id = joined.conn_id;
    let doc = joined.doc;
    let tx = joined.tx;
    let mut rx = joined.rx;

    // Sync step 1: tell the client our state vector so they can catch up.
    let sv = collab::state_vector(&doc);
    let _ = socket.send(Message::Binary(collab::encode_sync_step1(&sv))).await;

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Binary(raw))) => {
                        // Drop oversized frames before parsing/applying.
                        if raw.len() > state.opts.max_ws_message_bytes {
                            continue;
                        }
                        let Some((mtype, stype, payload)) = collab::parse_message(&raw) else {
                            continue;
                        };
                        if mtype == collab::MSG_SYNC {
                            if stype == collab::SYNC_STEP1 {
                                let diff = collab::diff_for(&doc, &payload);
                                let _ = socket
                                    .send(Message::Binary(collab::encode_sync_step2(&diff)))
                                    .await;
                            } else if stype == collab::SYNC_STEP2 || stype == collab::SYNC_UPDATE {
                                // Apply/persist/broadcast only for admins; a
                                // non-admin (Observer) frame is silently dropped.
                                if is_admin && collab::apply_update(&doc, &payload) {
                                    let state_bytes = collab::full_update(&doc);
                                    let _ = with_db(&state, move |conn| {
                                        if let Some(c) = conn {
                                            let _ = db::set_yjs_state(c, schedule_id, &state_bytes);
                                        }
                                    })
                                    .await;
                                    let _ = tx.send((conn_id, collab::encode_update_msg(&payload)));
                                }
                            }
                        } else if mtype == collab::MSG_AWARENESS {
                            let _ = tx.send((conn_id, raw));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // text/ping/pong — ignore
                    Some(Err(_)) => break,
                }
            }
            received = rx.recv() => {
                match received {
                    Ok((from, data)) => {
                        if from != conn_id
                            && socket.send(Message::Binary(data)).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {} // dropped some broadcasts; keep going
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    drop(rx);
    state.collab.leave_if_empty(schedule_id);
}

// ---------------------------------------------------------------------------
// Auth: login / logout / me
// ---------------------------------------------------------------------------

async fn api_login(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
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
    let password = v.get("password").and_then(|x| x.as_str()).unwrap_or("").to_string();

    let auth = state.auth.clone();
    let open = with_db(&state, move |conn| auth.is_open(conn)).await;
    if state.db.is_none() || open {
        let token = state.auth.new_session(boatsched_auth::OPEN_MODE_MEMBER);
        let label = if name.is_empty() { "instructor".to_string() } else { name };
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

async fn api_logout(headers: HeaderMap, State(state): State<SharedState>) -> Response {
    if let Some(token) = bsession(&headers) {
        state.auth.drop_session(&token);
    }
    let mut resp = Json(json!({ "ok": true })).into_response();
    if let Ok(hv) = HeaderValue::from_str("bsession=; Max-Age=0; Path=/") {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

async fn api_me(State(state): State<SharedState>, headers: HeaderMap) -> Response {
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

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ---------------------------------------------------------------------------
// Members
// ---------------------------------------------------------------------------

async fn list_members_api(State(state): State<SharedState>) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    match with_db(&state, |conn| db::list_members(conn.unwrap())).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => server_error("members", e),
    }
}

async fn members_progression(State(state): State<SharedState>) -> Response {
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
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
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
    match out {
        Ok(v) => Json(v).into_response(),
        Err(e) => server_error("progression", e),
    }
}

async fn rename_member_api(
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
    match with_db(&state, move |conn| db::rename_member(conn.unwrap(), member_id, &name)).await {
        Ok(()) => ok_true(),
        Err(db::DbError::Value(msg)) => e_error(StatusCode::CONFLICT, msg),
        Err(e) => server_error("rename_member", e),
    }
}

async fn delete_member_api(
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
    match with_db(&state, move |conn| db::delete_member(conn.unwrap(), member_id)).await {
        Ok(()) => ok_true(),
        Err(db::DbError::Value(msg)) => e_error(StatusCode::CONFLICT, msg),
        Err(e) => server_error("delete_member", e),
    }
}

async fn import_members_api(
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

// ---------------------------------------------------------------------------
// Set-admin
// ---------------------------------------------------------------------------

async fn set_instructor_admin(
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
    match with_db(&state, move |conn| db::set_admin(conn.unwrap(), member_id, is_admin)).await {
        Ok(()) => ok_true(),
        Err(e) => server_error("set_admin", e),
    }
}

// ---------------------------------------------------------------------------
// Student levels & export
// ---------------------------------------------------------------------------

async fn set_student_level(
    State(state): State<SharedState>,
    Path((enrollment_id, member_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may edit student levels"),
    };
    let v = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let level = match v.get("level") {
        Some(l) => json_str(l).trim().to_string(),
        None => return e_error(StatusCode::BAD_REQUEST, "'level'"),
    };
    if !VALID_LEVELS.contains(&level.as_str()) {
        return e_detail(
            StatusCode::BAD_REQUEST,
            format!("level must be one of {}", py_list(&VALID_LEVELS)),
        );
    }
    let level_c = level.clone();
    let updated = with_db(&state, move |conn| {
        let conn = conn.unwrap();
        // 404 unless the member is an enrolled, non-instructor student.
        if !db::is_student(conn, member_id, enrollment_id).unwrap_or(false) {
            return Ok::<bool, db::DbError>(false);
        }
        db::set_enrollment_level(conn, member_id, enrollment_id, &level_c)
    })
    .await;
    match updated {
        Ok(true) => Json(json!({ "ok": true, "level": level })).into_response(),
        Ok(false) => e_detail(StatusCode::NOT_FOUND, "student not found in this enrollment"),
        Err(e) => server_error("set_student_level", e),
    }
}

async fn export_students(
    State(state): State<SharedState>,
    Path(enrollment_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if state.db.is_none() {
        return e_error(StatusCode::SERVICE_UNAVAILABLE, "no database");
    }
    let _admin = match require_admin(&state, bsession(&headers)).await {
        Ok(c) => c,
        Err(_) => return e_detail(StatusCode::FORBIDDEN, "Only admins may export students"),
    };
    let students = match with_db(&state, move |conn| {
        db::list_enrolled_students(conn.unwrap(), enrollment_id)
    })
    .await
    {
        Ok(s) => s,
        Err(e) => return server_error("export_students", e),
    };
    // csv.writer uses \r\n terminators — match byte-for-byte.
    let mut csv = String::from("name,level\r\n");
    for (name, level) in students {
        csv.push_str(&csv_field(&name));
        csv.push(',');
        csv.push_str(&csv_field(&level));
        csv.push_str("\r\n");
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"students_season_{enrollment_id}.csv\""),
            ),
        ],
        csv,
    )
        .into_response()
}

/// Minimal RFC4180 field quoting matching Python's `csv.writer` defaults.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn api_health() -> Response {
    Json(json!({
        "status": "healthy",
        "timestamp": utc_now_iso(),
        "version": "1.0",
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Small ports of server.py module helpers
// ---------------------------------------------------------------------------

/// Port of `_extract_multipart`: pull the first file part's body out of a
/// multipart/form-data payload.
fn extract_multipart(raw: &[u8], ctype: &str) -> Vec<u8> {
    let boundary = match ctype.split("boundary=").last() {
        Some(b) => b.trim().trim_matches('"'),
        None => return raw.to_vec(),
    };
    let sep = format!("--{boundary}").into_bytes();
    for part in split_bytes(raw, &sep) {
        if let Some(idx) = find_sub(part, b"\r\n\r\n") {
            let head = &part[..idx];
            if find_sub(head, b"filename=").is_some() {
                let body = &part[idx + 4..];
                // strip the trailing CRLF before the next boundary
                let end = body
                    .windows(2)
                    .rposition(|w| w == b"\r\n")
                    .unwrap_or(body.len());
                return body[..end].to_vec();
            }
        }
    }
    raw.to_vec()
}

/// Port of `_parse_names_csv`: extract names from a CSV with a `name` column,
/// else treat each line's first comma-token as a name.
fn parse_names_csv(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    // Header with a "name" column?
    let header: Vec<String> = lines[0].split(',').map(|c| c.trim().to_lowercase()).collect();
    if let Some(col) = header.iter().position(|c| c == "name") {
        return lines[1..]
            .iter()
            .filter_map(|line| {
                line.split(',')
                    .nth(col)
                    .map(|c| c.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .collect();
    }
    // No 'name' header: first token of each line.
    lines
        .iter()
        .map(|line| line.trim().split(',').next().unwrap_or("").trim().trim_matches('"').to_string())
        .filter(|name| !name.is_empty() && !matches!(name.to_lowercase().as_str(), "name" | "naam"))
        .collect()
}

fn split_bytes<'a>(data: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + sep.len() <= data.len() {
        if &data[i..i + sep.len()] == sep {
            out.push(&data[start..i]);
            i += sep.len();
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&data[start..]);
    out
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

// ---------------------------------------------------------------------------
// JSON coercion helpers — mirror Python's permissive str()/int()/bool() coercion
// ---------------------------------------------------------------------------

/// Like Python `str(body[k])`: numbers/bools become their textual form.
fn json_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// Like Python `int(body[k])`: accept JSON ints and numeric strings.
fn json_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Like Python `bool(body[k])`: truthiness of common JSON shapes.
fn json_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map_or(false, |f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Render `["k", "b", "A", "B"]` the way Python's `list(...)` repr does, for the
/// invalid-level error message.
fn py_list(items: &[&str]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Best-effort UTC ISO-8601 timestamp (the health endpoint's `timestamp`; only
/// its presence is asserted). Avoids a chrono dependency.
fn utc_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (y + (m <= 2) as i64, m, d)
}
