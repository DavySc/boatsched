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
mod http;
mod routes;
mod upload;

use http::*;
use routes::{
    auth::*, eval::*, instructors::*, levels::*, members::*, misc::*, schedule::*, seasons::*,
    solve::*, ws::*,
};
use upload::*;

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
use serde_json::Value;

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
        .route("/api/skills", get(get_skills))
        .route("/api/progress", get(get_progress).post(post_progress))
        .route("/api/members/import", post(import_members_api))
        .route(
            "/api/members/:id",
            put(rename_member_api).delete(delete_member_api),
        )
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
