//! HTTP route parity tests for the axum server, mirroring the Python suites
//! (`test_server.py`, `test_auth.py`, `test_members.py`, `test_level_editing.py`,
//! `test_security.py`, `test_server_misc.py`). Requests go through the real
//! router via `tower::oneshot`; the shared `Arc` state means sessions and the DB
//! persist across requests within a test (so login flows work).

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use boatsched_core::{svec, Config};
use boatsched_db::schema::{create_tables, run_migrations};
use boatsched_server::{create_app, ServerOptions};
use rusqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

// ---- harness -------------------------------------------------------------

fn mem_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    run_migrations(&conn).unwrap();
    conn
}

fn app_no_db() -> Router {
    create_app(Config::default(), None, ServerOptions::default())
}

fn app_db() -> Router {
    create_app(Config::default(), Some(mem_conn()), ServerOptions::default())
}

fn app_db_opts(opts: ServerOptions) -> Router {
    create_app(Config::default(), Some(mem_conn()), opts)
}

/// Small config + db, for the solve flow.
fn app_solve() -> Router {
    let cfg = Config {
        boats: 2,
        levels: svec(&["k", "b"]),
        restricted: svec(&["Sat"]),
        diverse: svec(&["Fri"]),
        stage_seconds: 10.0,
        seconds: 10.0,
        ..Default::default()
    };
    create_app(cfg, Some(mem_conn()), ServerOptions::default())
}

struct Resp {
    status: StatusCode,
    cookie: Option<String>,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    json_body: Option<Value>,
) -> Resp {
    send_raw(
        app,
        method,
        uri,
        cookie,
        json_body.map(|v| ("application/json", serde_json::to_vec(&v).unwrap())),
    )
    .await
}

async fn send_raw(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<(&str, Vec<u8>)>,
) -> Resp {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, format!("bsession={c}"));
    }
    let req = match body {
        Some((ctype, bytes)) => b
            .header(header::CONTENT_TYPE, ctype)
            .body(Body::from(bytes))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            s.strip_prefix("bsession=")
                .map(|rest| rest.split(';').next().unwrap_or("").to_string())
        });
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
    Resp { status, cookie, body }
}

/// Create the first instructor (auto-admin, in open mode) and log in; returns
/// the admin session cookie.
async fn bootstrap_admin(app: &Router) -> String {
    let r = send(
        app,
        "POST",
        "/instructor",
        None,
        Some(json!({"name": "Admin", "password": "adminpw"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    login(app, "Admin", "adminpw").await
}

async fn create_nonadmin(app: &Router, admin: &str, name: &str, pw: &str) {
    let r = send(
        app,
        "POST",
        "/instructor",
        Some(admin),
        Some(json!({"name": name, "password": pw})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

async fn login(app: &Router, name: &str, pw: &str) -> String {
    let r = send(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({"name": name, "password": pw})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "login failed: {}", r.text());
    r.cookie.expect("login should set a bsession cookie")
}

// ---- config / health / public ------------------------------------------

#[tokio::test]
async fn config_reports_no_db() {
    let app = app_no_db();
    let r = send(&app, "GET", "/config", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    let j = r.json();
    assert_eq!(j["db"], json!(false));
    assert_eq!(j["boats"], json!(13));
    assert_eq!(j["levels"], json!(["k", "b", "A", "B"]));
}

#[tokio::test]
async fn config_reports_db_present() {
    let app = app_db();
    // open mode (no password) → config is reachable without auth
    let r = send(&app, "GET", "/config", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["db"], json!(true));
}

#[tokio::test]
async fn health_is_public_and_healthy() {
    let app = app_db();
    let r = send(&app, "GET", "/api/health", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["status"], json!("healthy"));
    assert!(r.json().get("timestamp").is_some());
}

#[tokio::test]
async fn security_headers_present() {
    let app = app_db();
    let r = send(&app, "GET", "/config", None, None).await;
    let app2 = app_db(); // headers also on a public route
    let _ = app2;
    // Re-send to inspect headers via a manual oneshot.
    let req = Request::builder()
        .uri("/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
    assert_eq!(resp.headers()["x-frame-options"], "DENY");
    assert_eq!(resp.headers()["referrer-policy"], "no-referrer");
    assert_eq!(r.status, StatusCode::OK);
}

// ---- open mode -----------------------------------------------------------

#[tokio::test]
async fn open_mode_allows_protected_routes_without_login() {
    let app = app_db(); // db but no password → open mode
    // /config is protected (not public) but open mode lets it through.
    assert_eq!(send(&app, "GET", "/config", None, None).await.status, StatusCode::OK);
    // open-mode caller is admin: can create a season with no cookie.
    let r = send(&app, "POST", "/seasons", None, Some(json!({"year": 2026}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["year"], json!(2026));
}

#[tokio::test]
async fn empty_lists_when_no_db() {
    let app = app_no_db();
    for path in ["/instructors", "/seasons", "/boats"] {
        let r = send(&app, "GET", path, None, None).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.json(), json!([]), "{path}");
    }
    // schedules with no season_id → []
    let r = send(&app, "GET", "/schedules", None, None).await;
    assert_eq!(r.json(), json!([]));
}

// ---- auth mode -----------------------------------------------------------

#[tokio::test]
async fn auth_mode_requires_login() {
    let app = app_db();
    bootstrap_admin(&app).await; // sets a password → auth mode
    // No cookie → 401 on a protected route.
    let r = send(&app, "GET", "/config", None, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_wrong_password_401_no_cookie() {
    let app = app_db();
    bootstrap_admin(&app).await;
    let r = send(
        &app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({"name": "Admin", "password": "wrong"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.cookie.is_none());
}

#[tokio::test]
async fn me_returns_identity_and_admin() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(&app, "GET", "/api/auth/me", Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["name"], json!("Admin"));
    assert_eq!(r.json()["is_admin"], json!(true));
}

#[tokio::test]
async fn logout_clears_session() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    assert_eq!(send(&app, "GET", "/config", Some(&admin), None).await.status, StatusCode::OK);
    send(&app, "POST", "/api/auth/logout", Some(&admin), None).await;
    // session dropped → now unauthorized
    assert_eq!(send(&app, "GET", "/config", Some(&admin), None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nonadmin_blocked_admin_routes() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    create_nonadmin(&app, &admin, "Bob", "bobpw").await;
    let bob = login(&app, "Bob", "bobpw").await;

    // create season → 403 admin required
    let r = send(&app, "POST", "/seasons", Some(&bob), Some(json!({"year": 2027}))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"], json!("admin required"));

    // swap → 403 with the boat-assignment detail string
    let r = send(
        &app,
        "POST",
        "/swap",
        Some(&bob),
        Some(json!({"schedule_id": 1, "day": "Sat", "name_a": "x", "name_b": "y"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["detail"], json!("Only admins may edit boat assignments"));
}

// ---- set-admin -----------------------------------------------------------

#[tokio::test]
async fn set_admin_promote_demote_and_self_guard() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    create_nonadmin(&app, &admin, "Bob", "bobpw").await;
    // Bob's member id: he's the 2nd member.
    let instr = send(&app, "GET", "/instructors", Some(&admin), None).await.json();
    let bob_id = instr
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == json!("Bob"))
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let admin_id = instr
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == json!("Admin"))
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    // promote Bob
    let r = send(
        &app,
        "POST",
        &format!("/api/instructors/{bob_id}/set-admin"),
        Some(&admin),
        Some(json!({"is_admin": true})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());

    // admin cannot remove own admin → 400
    let r = send(
        &app,
        "POST",
        &format!("/api/instructors/{admin_id}/set-admin"),
        Some(&admin),
        Some(json!({"is_admin": false})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], json!("cannot remove own admin status"));

    // set-admin on a non-instructor → 404
    let r = send(
        &app,
        "POST",
        "/api/instructors/9999/set-admin",
        Some(&admin),
        Some(json!({"is_admin": true})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ---- members -------------------------------------------------------------

#[tokio::test]
async fn member_import_list_rename_delete() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;

    // import 3 names via CSV
    let csv = b"name\nAlice\nBea\nCleo\n".to_vec();
    let r = send_raw(&app, "POST", "/api/members/import", Some(&admin), Some(("text/csv", csv))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["created"], json!(3));

    // list members
    let r = send(&app, "GET", "/api/members", Some(&admin), None).await;
    let members = r.json();
    let arr = members.as_array().unwrap();
    assert!(arr.iter().any(|m| m["name"] == json!("Alice")));
    let alice_id = arr.iter().find(|m| m["name"] == json!("Alice")).unwrap()["id"]
        .as_i64()
        .unwrap();

    // rename Alice → Alicia
    let r = send(
        &app,
        "PUT",
        &format!("/api/members/{alice_id}"),
        Some(&admin),
        Some(json!({"name": "Alicia"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    // duplicate rename → 409
    let r = send(
        &app,
        "PUT",
        &format!("/api/members/{alice_id}"),
        Some(&admin),
        Some(json!({"name": "Bea"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // delete → soft delete, disappears from list
    let r = send(&app, "DELETE", &format!("/api/members/{alice_id}"), Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = send(&app, "GET", "/api/members", Some(&admin), None).await;
    assert!(!r.json().as_array().unwrap().iter().any(|m| m["id"] == json!(alice_id)));
}

#[tokio::test]
async fn delete_instructor_member_conflicts() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let instr = send(&app, "GET", "/instructors", Some(&admin), None).await.json();
    let admin_id = instr.as_array().unwrap()[0]["id"].as_i64().unwrap();
    let r = send(&app, "DELETE", &format!("/api/members/{admin_id}"), Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn members_progression_shape() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(&app, "GET", "/api/members/progression", Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    let j = r.json();
    assert!(j.get("seasons").is_some());
    assert!(j.get("members").is_some());
    // Admin is an instructor → excluded from members.
    assert!(!j["members"].as_array().unwrap().iter().any(|m| m["name"] == json!("Admin")));
}

// ---- boats ---------------------------------------------------------------

#[tokio::test]
async fn boats_seeded_and_renamable() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(&app, "GET", "/boats", Some(&admin), None).await;
    assert_eq!(r.json().as_array().unwrap().len(), 13);
    let r = send(&app, "POST", "/boat", Some(&admin), Some(json!({"id": 1, "name": "Optimist"}))).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = send(&app, "GET", "/boats", Some(&admin), None).await;
    let boats = r.json();
    assert_eq!(boats.as_array().unwrap()[0]["name"], json!("Optimist"));
}

// ---- level editing -------------------------------------------------------

#[tokio::test]
async fn student_level_editing_and_validation() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    // season + an enrolled student
    let season = send(&app, "POST", "/seasons", Some(&admin), Some(json!({"year": 2026}))).await.json();
    let sid = season["id"].as_i64().unwrap();
    // import a member, then enroll via solve-less path: use member import + enrollment
    send_raw(&app, "POST", "/api/members/import", Some(&admin), Some(("text/csv", b"name\nSam\n".to_vec()))).await;
    let members = send(&app, "GET", "/api/members", Some(&admin), None).await.json();
    let sam_id = members.as_array().unwrap().iter().find(|m| m["name"] == json!("Sam")).unwrap()["id"].as_i64().unwrap();

    // not yet enrolled → 404
    let r = send(
        &app,
        "PATCH",
        &format!("/enrollment/{sid}/member/{sam_id}/level"),
        Some(&admin),
        Some(json!({"level": "A"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["detail"], json!("student not found in this enrollment"));
}

#[tokio::test]
async fn invalid_level_rejected() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(
        &app,
        "PATCH",
        "/enrollment/1/member/1/level",
        Some(&admin),
        Some(json!({"level": "z"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["detail"], json!("level must be one of ['k', 'b', 'A', 'B']"));
}

#[tokio::test]
async fn export_requires_admin() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    create_nonadmin(&app, &admin, "Bob", "bobpw").await;
    let bob = login(&app, "Bob", "bobpw").await;
    let r = send(&app, "GET", "/enrollment/1/students/export", Some(&bob), None).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["detail"], json!("Only admins may export students"));
    // admin gets a CSV with the header
    let r = send(&app, "GET", "/enrollment/1/students/export", Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text().starts_with("name,level\r\n"));
}

// ---- security: throttle, ttl, upload cap, password policy ---------------

#[tokio::test]
async fn login_rate_limited_after_max_failures() {
    let opts = ServerOptions { login_max_failures: 3, ..Default::default() };
    let app = app_db_opts(opts);
    bootstrap_admin(&app).await;
    for _ in 0..3 {
        let r = send(&app, "POST", "/api/auth/login", None, Some(json!({"name": "Admin", "password": "no"}))).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // even with the *correct* password now, the IP is blocked → 429
    let r = send(&app, "POST", "/api/auth/login", None, Some(json!({"name": "Admin", "password": "adminpw"}))).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn session_expires_after_ttl() {
    let opts = ServerOptions { session_ttl: std::time::Duration::from_millis(0), ..Default::default() };
    let app = app_db_opts(opts);
    let admin = bootstrap_admin(&app).await;
    // ttl 0 → the session is already expired on next use.
    let r = send(&app, "GET", "/config", Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_size_cap_enforced() {
    let opts = ServerOptions { max_upload_bytes: 16, ..Default::default() };
    let app = app_db_opts(opts);
    let admin = bootstrap_admin(&app).await;
    let big = vec![b'x'; 64];
    let r = send_raw(&app, "POST", "/api/members/import", Some(&admin), Some(("text/csv", big))).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn password_policy_min_length() {
    let opts = ServerOptions { min_password_len: 8, ..Default::default() };
    let app = app_db_opts(opts);
    // open mode: create_instructor checks admin (open → admin ok), then weak pw.
    let r = send(&app, "POST", "/instructor", None, Some(json!({"name": "A", "password": "short"}))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], json!("wachtwoord moet minstens 8 tekens zijn"));
}

#[tokio::test]
async fn duplicate_instructor_conflicts() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(&app, "POST", "/instructor", Some(&admin), Some(json!({"name": "Admin", "password": "x"}))).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.json()["error"], json!("already an instructor"));
}

// ---- solve + status flow -------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn solve_runs_and_persists() {
    let app = app_solve();
    let admin = bootstrap_admin(&app).await;
    let season = send(&app, "POST", "/seasons", Some(&admin), Some(json!({"year": 2026}))).await.json();
    let sid = season["id"].as_i64().unwrap();

    // roster: 2 k (new, auto-created) + 2 b (must pre-exist) → import the b's first.
    send_raw(&app, "POST", "/api/members/import", Some(&admin), Some(("text/csv", b"name\nBo\nBea\n".to_vec()))).await;
    let roster = b"name,level\nKa,k\nKi,k\nBo,b\nBea,b\n".to_vec();
    let r = send_raw(&app, "POST", &format!("/solve?season_id={sid}"), Some(&admin), Some(("text/csv", roster))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let jid = r.json()["id"].as_str().unwrap().to_string();

    // poll status until done
    let mut done = None;
    for _ in 0..100 {
        let r = send(&app, "GET", &format!("/status?id={jid}"), Some(&admin), None).await;
        let j = r.json();
        match j["status"].as_str() {
            Some("done") => { done = Some(j); break; }
            Some("error") => panic!("solve errored: {}", j),
            _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    let j = done.expect("solve did not finish in time");
    // 4 kids x 2 days = 8 result rows
    assert_eq!(j["result"].as_array().unwrap().len(), 8);
    assert!(j.get("schedule_id").is_some());
    assert_eq!(j["stats"]["mixed"], json!(0));

    // the schedule is now listable + readable
    let sched_id = j["schedule_id"].as_i64().unwrap();
    let r = send(&app, "GET", &format!("/schedules?season_id={sid}"), Some(&admin), None).await;
    assert!(r.json().as_array().unwrap().iter().any(|s| s["id"] == json!(sched_id)));
    let r = send(&app, "GET", &format!("/schedule?id={sched_id}"), Some(&admin), None).await;
    assert_eq!(r.json()["rows"].as_array().unwrap().len(), 8);
}

#[tokio::test]
async fn status_unknown_job_404() {
    let app = app_db();
    let admin = bootstrap_admin(&app).await;
    let r = send(&app, "GET", "/status?id=nope", Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn solve_rejects_bad_roster() {
    let app = app_solve();
    let admin = bootstrap_admin(&app).await;
    let r = send_raw(&app, "POST", "/solve", Some(&admin), Some(("text/csv", b"garbage with no columns".to_vec()))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}
