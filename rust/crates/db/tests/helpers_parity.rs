//! Parity tests for the data-access layer, mirroring `tests/test_db.py` and
//! the password-hashing cases from `tests/test_auth.py`.

use boatsched_core::{Kid, ScheduleRow};
use boatsched_db::{helpers, schema};
use rusqlite::{params, Connection};

fn fconn() -> Connection {
    let conn = boatsched_db::init_db(":memory:").unwrap();
    schema::run_migrations(&conn).unwrap();
    conn
}

fn kid(name: &str, level: &str) -> Kid {
    Kid::new(name, level)
}

fn member_names(conn: &Connection) -> std::collections::HashSet<String> {
    conn.prepare("SELECT name FROM member")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect()
}

fn id_of(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM member WHERE name=?1", params![name], |r| {
        r.get(0)
    })
    .unwrap()
}

fn members4() -> Vec<Kid> {
    vec![
        kid("Alice", "k"),
        kid("Bob", "k"),
        kid("Carol", "b"),
        kid("Dave", "b"),
    ]
}

fn result4() -> Vec<ScheduleRow> {
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
            name: "Carol".into(),
            level: "b".into(),
        },
        ScheduleRow {
            day: "Sat".into(),
            boat: 2,
            name: "Dave".into(),
            level: "b".into(),
        },
    ]
}

/// Pre-seed non-k returning members, create season, enroll all four.
fn seed_season(conn: &Connection, year: i64) -> i64 {
    let sid = helpers::create_season(conn, year).unwrap();
    for n in ["Carol", "Dave"] {
        conn.execute(
            "INSERT OR IGNORE INTO member (name) VALUES (?1)",
            params![n],
        )
        .unwrap();
    }
    helpers::add_or_update_members(conn, &members4(), Some(sid)).unwrap();
    sid
}

// ---- Insert / update ----
#[test]
fn insert_single_member() {
    let conn = fconn();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k")], None).unwrap();
    assert!(member_names(&conn).contains("Alice"));
}

#[test]
fn non_k_member_not_found_raises() {
    let conn = fconn();
    let err = helpers::add_or_update_members(&conn, &[kid("Ghost", "b")], None).unwrap_err();
    assert!(format!("{err}").contains("not found"));
}

#[test]
fn empty_list_inserts_nothing() {
    let conn = fconn();
    helpers::add_or_update_members(&conn, &[], None).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM member", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn no_duplicate_row_on_name_conflict() {
    let conn = fconn();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k")], None).unwrap();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k")], None).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM member WHERE name='Alice'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1);
}

// ---- Enrollment ----
#[test]
fn level_stored_and_updated_on_conflict() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 2026).unwrap();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k")], Some(sid)).unwrap();
    let lvl: String = conn
        .query_row(
            "SELECT e.level FROM enrollment e JOIN member m ON m.id=e.member_id WHERE m.name='Alice' AND e.season_id=?1",
            params![sid], |r| r.get(0)).unwrap();
    assert_eq!(lvl, "k");
    helpers::add_or_update_members(&conn, &[kid("Alice", "b")], Some(sid)).unwrap();
    let lvl: String = conn
        .query_row(
            "SELECT e.level FROM enrollment e JOIN member m ON m.id=e.member_id WHERE m.name='Alice' AND e.season_id=?1",
            params![sid], |r| r.get(0)).unwrap();
    assert_eq!(lvl, "b");
}

#[test]
fn no_enrollment_without_season_id() {
    let conn = fconn();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k")], None).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM enrollment", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn get_season_roster_returns_enrolled() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 2026).unwrap();
    conn.execute("INSERT INTO member (name) VALUES ('Bob')", [])
        .unwrap();
    helpers::add_or_update_members(&conn, &[kid("Alice", "k"), kid("Bob", "b")], Some(sid))
        .unwrap();
    let mut roster = helpers::get_season_roster(&conn, sid).unwrap();
    roster.sort();
    assert_eq!(
        roster,
        vec![("Alice".into(), "k".into()), ("Bob".into(), "b".into())]
    );
}

// ---- create_season / list_seasons ----
#[test]
fn create_season_duplicate_year_raises() {
    let conn = fconn();
    helpers::create_season(&conn, 2026).unwrap();
    assert!(helpers::create_season(&conn, 2026).is_err());
}

#[test]
fn list_seasons_newest_first() {
    let conn = fconn();
    for y in [2023, 2025, 2024] {
        helpers::create_season(&conn, y).unwrap();
    }
    let years: Vec<i64> = helpers::list_seasons(&conn)
        .unwrap()
        .iter()
        .map(|s| s.year)
        .collect();
    assert_eq!(years, vec![2025, 2024, 2023]);
}

// ---- save_schedule / get_schedule ----
#[test]
fn save_schedule_persists_and_returns_id() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let schedule_id = helpers::save_schedule(&conn, sid, "{\"boats\": 2}", &result4()).unwrap();
    assert!(schedule_id > 0);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM boat_assignment WHERE schedule_id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 4);
    let cfg: String = conn
        .query_row(
            "SELECT config FROM schedule WHERE id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cfg, "{\"boats\": 2}");
}

#[test]
fn save_schedule_unknown_member_raises() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let bad = vec![ScheduleRow {
        day: "Sat".into(),
        boat: 1,
        name: "Nobody".into(),
        level: "k".into(),
    }];
    let err = helpers::save_schedule(&conn, sid, "{}", &bad).unwrap_err();
    assert!(format!("{err}").contains("not found"));
}

#[test]
fn get_schedule_levels_from_enrollment() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let schedule_id = helpers::save_schedule(&conn, sid, "{}", &result4()).unwrap();
    let rows = helpers::get_schedule(&conn, schedule_id).unwrap();
    assert_eq!(rows.len(), 4);
    let lvl = |name: &str| rows.iter().find(|r| r.name == name).unwrap().level.clone();
    assert_eq!(lvl("Alice"), "k");
    assert_eq!(lvl("Carol"), "b");
}

#[test]
fn get_schedule_empty_for_unknown_id() {
    let conn = fconn();
    assert!(helpers::get_schedule(&conn, 9999).unwrap().is_empty());
}

#[test]
fn get_schedule_raises_on_missing_enrollment() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 2026).unwrap();
    conn.execute("INSERT INTO member (name) VALUES ('Alice')", [])
        .unwrap();
    let alice = id_of(&conn, "Alice");
    conn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?1, '{}')",
        params![sid],
    )
    .unwrap();
    let sch_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?1, ?2, 1, 'Sat')",
        params![sch_id, alice],
    ).unwrap();
    let err = helpers::get_schedule(&conn, sch_id).unwrap_err();
    assert!(format!("{err}").contains("not enrolled"));
}

// ---- list_schedules ----
#[test]
fn list_schedules_newest_first_and_isolated() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let id1 = helpers::save_schedule(&conn, sid, "{}", &result4()).unwrap();
    let id2 = helpers::save_schedule(&conn, sid, "{}", &result4()).unwrap();
    let ids: Vec<i64> = helpers::list_schedules(&conn, sid)
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec![id2, id1]);
    let sid2 = helpers::create_season(&conn, 2027).unwrap();
    assert!(helpers::list_schedules(&conn, sid2).unwrap().is_empty());
}

// ---- swap_assignment ----
#[test]
fn swap_exchanges_and_reversible_and_noop() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let schedule_id = helpers::save_schedule(&conn, sid, "{}", &result4()).unwrap();
    let boat_of = |name: &str| -> i64 {
        conn.query_row(
            "SELECT ba.boat_id FROM boat_assignment ba JOIN member m ON m.id=ba.member_id \
             WHERE ba.schedule_id=?1 AND ba.day='Sat' AND m.name=?2",
            params![schedule_id, name],
            |r| r.get(0),
        )
        .unwrap()
    };
    let a_before = boat_of("Alice");
    let c_before = boat_of("Carol");
    assert_ne!(a_before, c_before);
    helpers::swap_assignment(&conn, schedule_id, "Sat", "Alice", "Carol").unwrap();
    assert_eq!(boat_of("Alice"), c_before);
    assert_eq!(boat_of("Carol"), a_before);
    // reversible
    helpers::swap_assignment(&conn, schedule_id, "Sat", "Alice", "Carol").unwrap();
    assert_eq!(boat_of("Alice"), a_before);
    // same boat no-op (Alice & Bob share boat 1)
    let before = boat_of("Alice");
    helpers::swap_assignment(&conn, schedule_id, "Sat", "Alice", "Bob").unwrap();
    assert_eq!(boat_of("Alice"), before);
}

#[test]
fn swap_unknown_member_raises() {
    let conn = fconn();
    let sid = seed_season(&conn, 2026);
    let schedule_id = helpers::save_schedule(&conn, sid, "{}", &result4()).unwrap();
    let err = helpers::swap_assignment(&conn, schedule_id, "Sat", "Alice", "Nobody").unwrap_err();
    assert!(format!("{err}").contains("not found"));
}

// ---- boats ----
#[test]
fn rename_boat_set_and_clear() {
    let conn = fconn();
    helpers::rename_boat(&conn, 1, Some("Blue Jay")).unwrap();
    let boats = helpers::list_boats(&conn).unwrap();
    assert_eq!(boats.len(), 13);
    assert_eq!(
        boats.iter().find(|b| b.id == 1).unwrap().name.as_deref(),
        Some("Blue Jay")
    );
    helpers::rename_boat(&conn, 1, Some("")).unwrap(); // empty clears
    let boats = helpers::list_boats(&conn).unwrap();
    assert_eq!(boats.iter().find(|b| b.id == 1).unwrap().name, None);
    helpers::rename_boat(&conn, 1, Some("X")).unwrap();
    helpers::rename_boat(&conn, 1, None).unwrap(); // None clears
    let boats = helpers::list_boats(&conn).unwrap();
    assert_eq!(boats.iter().find(|b| b.id == 1).unwrap().name, None);
}

// ---- seed_members ----
#[test]
fn seed_members_counts_and_trimming() {
    let conn = fconn();
    let r = helpers::seed_members(&conn, &["Alpha", "Beta"]).unwrap();
    assert_eq!(r.created, 2);
    let r2 = helpers::seed_members(&conn, &["Alpha", "Gamma"]).unwrap();
    assert_eq!(r2.created, 1);
    assert_eq!(r2.existing, 1);
    // blank/empty filtered, whitespace trimmed
    let r3 = helpers::seed_members(&conn, &["  ", "", "  Trimmed  "]).unwrap();
    assert_eq!(r3.created, 1);
    assert!(member_names(&conn).contains("Trimmed"));
    assert!(!member_names(&conn).contains("  Trimmed  "));
    let empty = helpers::seed_members(&conn, &[]).unwrap();
    assert_eq!(empty.created, 0);
    assert_eq!(empty.existing, 0);
}

// ---- list_members ----
#[test]
fn list_members_active_ordered_excludes_inactive() {
    let conn = fconn();
    for n in ["Zara", "Anna"] {
        conn.execute("INSERT INTO member (name) VALUES (?1)", params![n])
            .unwrap();
    }
    conn.execute("INSERT INTO member (name, active) VALUES ('Gone', 0)", [])
        .unwrap();
    let names: Vec<String> = helpers::list_members(&conn)
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, vec!["Anna".to_string(), "Zara".to_string()]);
    assert!(!names.contains(&"Gone".to_string()));
}

// ---- delete_member / rename_member ----
#[test]
fn delete_member_soft_and_blocks_instructor() {
    let conn = fconn();
    conn.execute("INSERT INTO member (name) VALUES ('Del')", [])
        .unwrap();
    let mid = id_of(&conn, "Del");
    helpers::delete_member(&conn, mid).unwrap();
    let active: i64 = conn
        .query_row("SELECT active FROM member WHERE id=?1", params![mid], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(active, 0);

    conn.execute("INSERT INTO member (name) VALUES ('Inst')", [])
        .unwrap();
    let imid = id_of(&conn, "Inst");
    conn.execute(
        "INSERT INTO instructor (member_id) VALUES (?1)",
        params![imid],
    )
    .unwrap();
    let err = helpers::delete_member(&conn, imid).unwrap_err();
    assert!(format!("{err}").contains("instructeur"));
}

#[test]
fn rename_member_updates_empty_and_collision() {
    let conn = fconn();
    conn.execute("INSERT INTO member (name) VALUES ('Old')", [])
        .unwrap();
    conn.execute("INSERT INTO member (name) VALUES ('Exists')", [])
        .unwrap();
    let mid = id_of(&conn, "Old");
    helpers::rename_member(&conn, mid, "New").unwrap();
    assert!(member_names(&conn).contains("New"));
    let err = helpers::rename_member(&conn, mid, "   ").unwrap_err();
    assert!(format!("{err}").contains("empty"));
    let err = helpers::rename_member(&conn, mid, "Exists").unwrap_err();
    assert!(format!("{err}").contains("bestaat"));
}

// ---- set_admin ----
#[test]
fn set_admin_promote_demote() {
    let conn = fconn();
    conn.execute("INSERT INTO member (name) VALUES ('I')", [])
        .unwrap();
    let mid = id_of(&conn, "I");
    conn.execute(
        "INSERT INTO instructor (member_id, is_admin) VALUES (?1, 0)",
        params![mid],
    )
    .unwrap();
    helpers::set_admin(&conn, mid, true).unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT is_admin FROM instructor WHERE member_id=?1",
            params![mid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, 1);
    helpers::set_admin(&conn, mid, false).unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT is_admin FROM instructor WHERE member_id=?1",
            params![mid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, 0);
}

// ---- yjs state ----
#[test]
fn yjs_state_roundtrip() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 9001).unwrap();
    conn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?1, '{}')",
        params![sid],
    )
    .unwrap();
    let sch_id = conn.last_insert_rowid();
    assert_eq!(helpers::get_yjs_state(&conn, sch_id).unwrap(), None);
    let payload = b"\x00\x01\x02state";
    helpers::set_yjs_state(&conn, sch_id, payload).unwrap();
    assert_eq!(
        helpers::get_yjs_state(&conn, sch_id).unwrap().as_deref(),
        Some(payload.as_slice())
    );
    assert_eq!(helpers::get_yjs_state(&conn, 99999).unwrap(), None);
}

// ---- start_date ----
#[test]
fn start_date_roundtrip_and_overwrite() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 9002).unwrap();
    conn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?1, '{}')",
        params![sid],
    )
    .unwrap();
    let sch_id = conn.last_insert_rowid();
    assert_eq!(
        helpers::get_schedule_start_date(&conn, sch_id).unwrap(),
        None
    );
    helpers::set_schedule_start_date(&conn, sch_id, Some("2099-07-01")).unwrap();
    assert_eq!(
        helpers::get_schedule_start_date(&conn, sch_id)
            .unwrap()
            .as_deref(),
        Some("2099-07-01")
    );
    helpers::set_schedule_start_date(&conn, sch_id, Some("2099-08-01")).unwrap();
    assert_eq!(
        helpers::get_schedule_start_date(&conn, sch_id)
            .unwrap()
            .as_deref(),
        Some("2099-08-01")
    );
}

// ---- replace_assignments ----
#[test]
fn replace_assignments_replaces_and_unknown_raises() {
    let conn = fconn();
    let sid = helpers::create_season(&conn, 9003).unwrap();
    for n in ["Alice", "Bob"] {
        conn.execute("INSERT INTO member (name) VALUES (?1)", params![n])
            .unwrap();
        let mid = id_of(&conn, n);
        conn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (?1, ?2, 'k')",
            params![mid, sid],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?1, '{}')",
        params![sid],
    )
    .unwrap();
    let sch_id = conn.last_insert_rowid();
    for n in ["Alice", "Bob"] {
        let mid = id_of(&conn, n);
        conn.execute("INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?1, ?2, 1, 'Sat')", params![sch_id, mid]).unwrap();
    }
    helpers::replace_assignments(
        &conn,
        sch_id,
        &[
            ("Alice".into(), "Sat".into(), 2),
            ("Bob".into(), "Sat".into(), 2),
        ],
    )
    .unwrap();
    let all_two: bool = conn
        .prepare("SELECT boat_id FROM boat_assignment WHERE schedule_id=?1")
        .unwrap()
        .query_map(params![sch_id], |r| r.get::<_, i64>(0))
        .unwrap()
        .filter_map(Result::ok)
        .all(|b| b == 2);
    assert!(all_two);
    let err = helpers::replace_assignments(&conn, sch_id, &[("NoSuch".into(), "Sat".into(), 1)])
        .unwrap_err();
    assert!(format!("{err}").contains("not found"));
}

// ---- password hashing (mirrors TestPasswordHashing) ----
fn make_instructor(conn: &Connection, name: &str) -> i64 {
    helpers::add_or_update_members(conn, &[kid(name, "k")], None).unwrap();
    let mid = id_of(conn, name);
    conn.execute(
        "INSERT OR IGNORE INTO instructor (member_id) VALUES (?1)",
        params![mid],
    )
    .unwrap();
    mid
}

#[test]
fn password_hash_not_plaintext_and_lengths() {
    let conn = fconn();
    let mid = make_instructor(&conn, "Hank");
    helpers::set_instructor_password(&conn, mid, "plaintextpw").unwrap();
    let (ph, ps): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT password_hash, password_salt FROM instructor WHERE member_id=?1",
            params![mid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(ph.len(), 64);
    assert_eq!(ps.len(), 32);
    assert!(!ph.windows(11).any(|w| w == b"plaintextpw"));
}

#[test]
fn password_verify_correct_wrong_and_distinct_salts() {
    let conn = fconn();
    let m1 = make_instructor(&conn, "Kate");
    let m2 = make_instructor(&conn, "Leo");
    helpers::set_instructor_password(&conn, m1, "shared").unwrap();
    helpers::set_instructor_password(&conn, m2, "shared").unwrap();
    assert_eq!(
        helpers::verify_instructor_login(&conn, "Kate", "shared").unwrap(),
        Some(m1)
    );
    assert_eq!(
        helpers::verify_instructor_login(&conn, "Kate", "wrong").unwrap(),
        None
    );
    // same password, different salt → different stored hash
    let h1: Vec<u8> = conn
        .query_row(
            "SELECT password_hash FROM instructor WHERE member_id=?1",
            params![m1],
            |r| r.get(0),
        )
        .unwrap();
    let h2: Vec<u8> = conn
        .query_row(
            "SELECT password_hash FROM instructor WHERE member_id=?1",
            params![m2],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(h1, h2);
}

#[test]
fn login_is_case_insensitive_on_name() {
    let conn = fconn();
    let mid = make_instructor(&conn, "Alice");
    helpers::set_instructor_password(&conn, mid, "secret123").unwrap();
    // NOCASE name match
    assert_eq!(
        helpers::verify_instructor_login(&conn, "alice", "secret123").unwrap(),
        Some(mid)
    );
}

// ---- ensure_admin (mirrors TestEnsureAdmin) ----
#[test]
fn ensure_admin_creates_idempotent_promotes_empty() {
    let conn = fconn();
    let created = helpers::ensure_admin(&conn, "Boss", "bosspw1234").unwrap();
    assert!(created);
    assert!(
        helpers::verify_instructor_login(&conn, "Boss", "bosspw1234")
            .unwrap()
            .is_some()
    );
    let is_admin: i64 = conn.query_row(
        "SELECT is_admin FROM instructor i JOIN member m ON m.id=i.member_id WHERE m.name='Boss'", [], |r| r.get(0)).unwrap();
    assert_eq!(is_admin, 1);
    // idempotent: second call with different pw keeps original
    let created2 = helpers::ensure_admin(&conn, "Boss", "different").unwrap();
    assert!(!created2);
    assert!(
        helpers::verify_instructor_login(&conn, "Boss", "bosspw1234")
            .unwrap()
            .is_some()
    );
    assert!(helpers::verify_instructor_login(&conn, "Boss", "different")
        .unwrap()
        .is_none());
    // promotes existing member without duplicate row
    conn.execute("INSERT INTO member (name) VALUES ('Existing')", [])
        .unwrap();
    helpers::ensure_admin(&conn, "Existing", "newadminpw1").unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM member WHERE name='Existing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    // empty password raises
    assert!(helpers::ensure_admin(&conn, "X", "").is_err());
}

// ---- list_instructors flags ----
#[test]
fn list_instructors_password_flags() {
    let conn = fconn();
    let alice = make_instructor(&conn, "Alice");
    helpers::set_instructor_password(&conn, alice, "secret123").unwrap();
    make_instructor(&conn, "Frank"); // no password
    let rows = helpers::list_instructors(&conn).unwrap();
    let by_name: std::collections::HashMap<_, _> =
        rows.iter().map(|r| (r.name.clone(), r)).collect();
    assert_eq!(by_name["Alice"].has_password, 1);
    assert_eq!(by_name["Frank"].has_password, 0);
}
