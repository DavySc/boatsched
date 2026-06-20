//! Yjs collaborative-editing parity tests, mirroring `tests/test_collab.py`
//! (TestWebSocketSync, TestTwoClientConvergence, TestCommitEndpoint).
//!
//! WebSocket tests run a real axum server on an ephemeral port and drive it
//! with a `tokio-tungstenite` client + a `yrs` doc (the analogue of the Python
//! tests' pycrdt client). Commit tests use `tower::oneshot` and seed the doc
//! state directly into the DB. All run in open mode (db present, no password),
//! so a cookieless client is an admin editor — no HTTP login needed.

use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use boatsched_core::{Config, ScheduleRow};
use boatsched_db::schema::{create_tables, run_migrations};
use boatsched_server::collab;
use boatsched_server::{create_app, ServerOptions};
use futures_util::{SinkExt, StreamExt};
use rusqlite::Connection;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use yrs::{Array, ArrayPrelim, Doc, Map, Transact};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ---- schedule rows -------------------------------------------------------

fn valid_rows() -> Vec<ScheduleRow> {
    // 4 kids x 2 days = 8 rows, cap 2, 2 boats, no (name,day) duplicate.
    let mut rows = Vec::new();
    for day in ["Sat", "Fri"] {
        rows.push(ScheduleRow {
            day: day.into(),
            boat: 1,
            name: "Alice".into(),
            level: "k".into(),
        });
        rows.push(ScheduleRow {
            day: day.into(),
            boat: 1,
            name: "Bob".into(),
            level: "k".into(),
        });
        rows.push(ScheduleRow {
            day: day.into(),
            boat: 2,
            name: "Carol".into(),
            level: "b".into(),
        });
        rows.push(ScheduleRow {
            day: day.into(),
            boat: 2,
            name: "Dave".into(),
            level: "b".into(),
        });
    }
    rows
}

/// Doc rows with Alice in two Sat boats → a (name, day) conflict.
fn conflicting_rows() -> Vec<ScheduleRow> {
    vec![
        ScheduleRow {
            day: "Sat".into(),
            boat: 1,
            name: "Alice".into(),
            level: "k".into(),
        },
        ScheduleRow {
            day: "Sat".into(),
            boat: 1,
            name: "Bob".into(),
            level: "k".into(),
        },
        ScheduleRow {
            day: "Sat".into(),
            boat: 2,
            name: "Alice".into(),
            level: "k".into(),
        },
        ScheduleRow {
            day: "Sat".into(),
            boat: 2,
            name: "Dave".into(),
            level: "b".into(),
        },
    ]
}

/// Build a db with members + a saved schedule; `doc_rows` seeds `yjs_state`
/// (which may differ from the saved assignments, e.g. to inject a conflict).
fn seed(doc_rows: &[ScheduleRow]) -> (Connection, i64) {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    run_migrations(&conn).unwrap();
    for n in ["Alice", "Bob", "Carol", "Dave"] {
        conn.execute("INSERT INTO member (name) VALUES (?)", [n])
            .unwrap();
    }
    let season = boatsched_db::create_season(&conn, 2300).unwrap();
    // Enroll so get_schedule (which requires a per-season level) can read back.
    for (mid, lvl) in [(1, "k"), (2, "k"), (3, "b"), (4, "b")] {
        boatsched_db::add_enrollment(&conn, mid, season, lvl).unwrap();
    }
    let sid = boatsched_db::save_schedule(&conn, season, "{}", &valid_rows()).unwrap();
    let yjs = collab::init_doc_update_from_rows(doc_rows);
    boatsched_db::set_yjs_state(&conn, sid, &yjs).unwrap();
    (conn, sid)
}

fn app_for(conn: Connection) -> Router {
    create_app(Config::default(), Some(conn), ServerOptions::default())
}

// ---- HTTP (oneshot) helper for commit tests ------------------------------

async fn post(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

// ---- WS helpers ----------------------------------------------------------

async fn start_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr.to_string()
}

async fn connect(addr: &str, sid: i64) -> Ws {
    let url = format!("ws://{addr}/collab/{sid}");
    let (ws, _resp) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws
}

async fn recv_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws recv timed out")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Binary(b) = msg {
            return b;
        }
    }
}

/// Drain the server's step1, send our step1 (empty SV), apply the step2 reply.
async fn full_sync(ws: &mut Ws) -> Doc {
    let _server_step1 = recv_binary(ws).await; // server's initial step1
    let doc = Doc::new();
    doc.get_or_insert_map("schedule");
    let sv = collab::state_vector(&doc);
    ws.send(Message::Binary(collab::encode_sync_step1(&sv)))
        .await
        .unwrap();
    let reply = recv_binary(ws).await;
    let (_, st, payload) = collab::parse_message(&reply).unwrap();
    assert_eq!(st, collab::SYNC_STEP2);
    assert!(collab::apply_update(&doc, &payload));
    doc
}

fn schedule_keys(doc: &Doc) -> Vec<String> {
    let sched = doc.get_or_insert_map("schedule");
    let txn = doc.transact();
    sched.iter(&txn).map(|(k, _)| k.to_string()).collect()
}

fn total_members(doc: &Doc) -> usize {
    let sched = doc.get_or_insert_map("schedule");
    let txn = doc.transact();
    sched
        .iter(&txn)
        .map(|(_, v)| {
            v.cast::<yrs::ArrayRef>()
                .map(|a| a.len(&txn) as usize)
                .unwrap_or(0)
        })
        .sum()
}

fn has_key(doc: &Doc, key: &str) -> bool {
    let sched = doc.get_or_insert_map("schedule");
    let txn = doc.transact();
    sched.get(&txn, key).is_some()
}

/// Add `sched["key"] = [val]` and return the incremental v1 update.
fn make_edit(doc: &Doc, key: &str, val: &str) -> Vec<u8> {
    let sv_before = collab::state_vector(doc);
    let sched = doc.get_or_insert_map("schedule");
    {
        let mut txn = doc.transact_mut();
        sched.insert(&mut txn, key, ArrayPrelim::from(vec![val.to_string()]));
    }
    collab::diff_for(doc, &sv_before)
}

// ---- TestWebSocketSync ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_sends_step1_on_connect() {
    let (conn, sid) = seed(&valid_rows());
    let addr = start_server(app_for(conn)).await;
    let mut ws = connect(&addr, sid).await;
    let msg = recv_binary(&mut ws).await;
    let (m, s, sv) = collab::parse_message(&msg).unwrap();
    assert_eq!(m, collab::MSG_SYNC);
    assert_eq!(s, collab::SYNC_STEP1);
    assert!(!sv.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn step1_gets_step2_and_populates_doc() {
    let (conn, sid) = seed(&valid_rows());
    let addr = start_server(app_for(conn)).await;
    let mut ws = connect(&addr, sid).await;
    let doc = full_sync(&mut ws).await;
    let keys = schedule_keys(&doc);
    assert!(!keys.is_empty());
    assert!(keys.iter().all(|k| k.contains(':')));
    assert_eq!(total_members(&doc), 8); // 4 kids x 2 days
}

// ---- TestTwoClientConvergence -------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_from_one_client_reaches_other() {
    let (conn, sid) = seed(&valid_rows());
    let addr = start_server(app_for(conn)).await;
    let mut ws1 = connect(&addr, sid).await;
    let mut ws2 = connect(&addr, sid).await;
    let doc1 = full_sync(&mut ws1).await;
    let doc2 = full_sync(&mut ws2).await;

    let incremental = make_edit(&doc1, "_sentinel", "marker");
    ws1.send(Message::Binary(collab::encode_update_msg(&incremental)))
        .await
        .unwrap();

    // ws2 receives a SYNC_UPDATE broadcast; apply and verify convergence.
    let broadcast = recv_binary(&mut ws2).await;
    let (m, s, payload) = collab::parse_message(&broadcast).unwrap();
    assert_eq!(m, collab::MSG_SYNC);
    assert_eq!(s, collab::SYNC_UPDATE);
    assert!(collab::apply_update(&doc2, &payload));
    assert!(has_key(&doc2, "_sentinel"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_persisted_and_late_join_sees_it() {
    let (conn, sid) = seed(&valid_rows());
    let addr = start_server(app_for(conn)).await;

    // Client 1 edits, then disconnects (room freed, state persisted to DB).
    {
        let mut ws1 = connect(&addr, sid).await;
        let doc1 = full_sync(&mut ws1).await;
        let incremental = make_edit(&doc1, "_late_key", "late_marker");
        ws1.send(Message::Binary(collab::encode_update_msg(&incremental)))
            .await
            .unwrap();
        // give the server a moment to apply + persist before closing
        tokio::time::sleep(Duration::from_millis(200)).await;
        ws1.close(None).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client 2 connects fresh → the room reloads from DB and includes the edit.
    let mut ws2 = connect(&addr, sid).await;
    let doc2 = full_sync(&mut ws2).await;
    assert!(has_key(&doc2, "_late_key"));
}

// ---- cross-implementation wire compatibility -----------------------------

/// A Yjs **v1** update produced by **pycrdt** (the Python side), captured via
/// `rust/bench`-style one-liner. Doc: schedule = {Sat:1=[Alice,Bob],
/// Sat:2=[Carol], Fri:1=[Alice]}. Decoding it here proves pycrdt↔yrs wire
/// compatibility, not merely yrs↔yrs.
const PYCRDT_UPDATE: &[u8] = &[
    1, 6, 189, 176, 206, 156, 159, 140, 215, 4, 0, 39, 1, 8, 115, 99, 104, 101, 100, 117, 108, 101,
    5, 83, 97, 116, 58, 49, 0, 8, 0, 189, 176, 206, 156, 159, 140, 215, 4, 0, 2, 119, 5, 65, 108,
    105, 99, 101, 119, 3, 66, 111, 98, 39, 1, 8, 115, 99, 104, 101, 100, 117, 108, 101, 5, 83, 97,
    116, 58, 50, 0, 8, 0, 189, 176, 206, 156, 159, 140, 215, 4, 3, 1, 119, 5, 67, 97, 114, 111,
    108, 39, 1, 8, 115, 99, 104, 101, 100, 117, 108, 101, 5, 70, 114, 105, 58, 49, 0, 8, 0, 189,
    176, 206, 156, 159, 140, 215, 4, 5, 1, 119, 5, 65, 108, 105, 99, 101, 0,
];

#[tokio::test]
async fn pycrdt_update_decodes_in_yrs() {
    let doc = Doc::new();
    doc.get_or_insert_map("schedule");
    assert!(
        collab::apply_update(&doc, PYCRDT_UPDATE),
        "yrs should decode the pycrdt v1 update"
    );

    let mut keys = schedule_keys(&doc);
    keys.sort();
    assert_eq!(keys, vec!["Fri:1", "Sat:1", "Sat:2"]);
    assert_eq!(total_members(&doc), 4); // Alice,Bob + Carol + Alice

    // commit-style read: Alice appears Sat:1 and Fri:1 (different days → no
    // conflict); the assignment set resolves cleanly.
    let assignments = collab::read_assignments(&doc).expect("no conflict");
    assert_eq!(assignments.len(), 4);
}

// ---- TestCommitEndpoint --------------------------------------------------

#[tokio::test]
async fn commit_no_db_returns_503() {
    let app = create_app(Config::default(), None, ServerOptions::default());
    let (status, _) = post(&app, "/schedule/1/commit").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn commit_valid_state_returns_ok_and_preserves_rows() {
    let (conn, sid) = seed(&valid_rows());
    let app = app_for(conn);
    let (status, body) = post(&app, &format!("/schedule/{sid}/commit")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));

    // the schedule still reads back 8 rows
    let req = Request::builder()
        .uri(format!("/schedule?id={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let rb = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&rb).unwrap();
    assert_eq!(v["rows"].as_array().unwrap().len(), 8);
}

#[tokio::test]
async fn commit_detects_conflict() {
    let (conn, sid) = seed(&conflicting_rows());
    let app = app_for(conn);
    let (status, body) = post(&app, &format!("/schedule/{sid}/commit")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(false));
    let conflicts = body["conflicts"].as_array().unwrap();
    assert!(!conflicts.is_empty());
    let c = &conflicts[0];
    assert!(c.get("name").is_some() && c.get("day").is_some() && c.get("boats").is_some());
    assert_eq!(c["name"], json!("Alice"));
    assert_eq!(c["day"], json!("Sat"));
}
