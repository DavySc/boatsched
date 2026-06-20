//! SQLite connection management + schema. Mirrors `db.py`.
//!
//! Stage (a) scope: connection PRAGMAs, schema creation, and the idempotent
//! migrations. The member/season/schedule/instructor data helpers are added in
//! stage (b).

pub mod eval_catalog;
pub mod helpers;
pub mod schema;

pub use helpers::*;

use rusqlite::Connection;

/// Open a connection with the same PRAGMAs as Python `get_connection`:
/// `journal_mode=WAL` and `foreign_keys=ON`.
pub fn get_connection(db_path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    // pragma_update would error on the WAL return row; query_row reads it back.
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Open + create all tables (mirrors `init_db`).
pub fn init_db(db_path: &str) -> rusqlite::Result<Connection> {
    let conn = get_connection(db_path)?;
    schema::create_tables(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        init_db(":memory:").unwrap()
    }

    #[test]
    fn thirteen_boats_seeded() {
        let conn = mem();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM boat", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 13);
    }

    #[test]
    fn boat_ids_are_one_to_thirteen() {
        let conn = mem();
        let ids: Vec<i64> = conn
            .prepare("SELECT boat_id FROM boat ORDER BY boat_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(ids, (1..=13).collect::<Vec<_>>());
    }

    #[test]
    fn member_has_no_level_column() {
        let conn = mem();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(member)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(!cols.contains(&"level".to_string()));
    }

    #[test]
    fn enrollment_level_check_rejects_invalid() {
        let conn = mem();
        conn.execute("INSERT INTO member (name) VALUES ('A')", [])
            .unwrap();
        conn.execute("INSERT INTO season (year) VALUES (2026)", [])
            .unwrap();
        let res = conn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (1, 1, 'bad')",
            [],
        );
        assert!(res.is_err(), "CHECK constraint should reject 'bad'");
    }

    // ---- migration idempotency (mirrors TestMigrationIdempotency) ----
    #[test]
    fn add_admin_column_is_idempotent() {
        let conn = mem();
        schema::add_admin_column(&conn).unwrap();
        schema::add_admin_column(&conn).unwrap(); // second call no-ops
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(instructor)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"is_admin".to_string()));
    }

    #[test]
    fn add_yjs_and_start_date_idempotent() {
        let conn = mem();
        schema::add_yjs_state_column(&conn).unwrap();
        schema::add_yjs_state_column(&conn).unwrap();
        schema::add_start_date_column(&conn).unwrap();
        schema::add_start_date_column(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(schedule)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"yjs_state".to_string()));
        assert!(cols.contains(&"start_date".to_string()));
    }

    #[test]
    fn auto_grant_skips_when_admin_exists() {
        let conn = mem();
        schema::add_admin_column(&conn).unwrap();
        conn.execute("INSERT INTO member (name) VALUES ('Root')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO instructor (member_id, is_admin) VALUES (1, 1)",
            [],
        )
        .unwrap();
        schema::auto_grant_first_admin(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM instructor WHERE is_admin=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn auto_grant_promotes_all_when_none() {
        let conn = mem();
        schema::add_admin_column(&conn).unwrap();
        for (i, name) in ["I1", "I2"].iter().enumerate() {
            conn.execute("INSERT INTO member (name) VALUES (?1)", [name])
                .unwrap();
            conn.execute(
                "INSERT INTO instructor (member_id, is_admin) VALUES (?1, 0)",
                [(i as i64) + 1],
            )
            .unwrap();
        }
        schema::auto_grant_first_admin(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM instructor WHERE is_admin=0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn make_name_index_nocase_is_idempotent_and_enforces() {
        let conn = mem();
        schema::make_name_index_nocase(&conn).unwrap();
        schema::make_name_index_nocase(&conn).unwrap(); // no-op second time
        conn.execute("INSERT INTO member (name) VALUES ('Van Aarle')", [])
            .unwrap();
        // NOCASE unique index: inserting a case-variant must conflict.
        let dup = conn.execute("INSERT INTO member (name) VALUES ('van aarle')", []);
        assert!(
            dup.is_err(),
            "NOCASE unique index should reject case-variant dup"
        );
    }

    #[test]
    fn make_name_index_nocase_errors_on_case_collision() {
        let conn = mem();
        // Insert collision under the default BINARY unique index (distinct).
        conn.execute("INSERT INTO member (name) VALUES ('Bob')", [])
            .unwrap();
        conn.execute("INSERT INTO member (name) VALUES ('bob')", [])
            .unwrap();
        match schema::make_name_index_nocase(&conn) {
            Err(schema::MigrationError::NameCollision(_)) => {}
            other => panic!("expected NameCollision, got {other:?}"),
        }
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let conn = mem();
        schema::run_migrations(&conn).unwrap();
        schema::run_migrations(&conn).unwrap();
    }

    // ---- evaluation tables + progress ----------------------------------
    use std::collections::BTreeMap;

    fn mem_migrated() -> Connection {
        let conn = init_db(":memory:").unwrap();
        schema::run_migrations(&conn).unwrap();
        conn
    }

    /// Seed a student (member 1), an instructor (member 2), and season 1.
    fn seed_student_and_instructor(conn: &Connection) {
        conn.execute("INSERT INTO member (name) VALUES ('Kid')", []).unwrap();
        conn.execute("INSERT INTO member (name) VALUES ('Instr')", []).unwrap();
        conn.execute("INSERT INTO season (year) VALUES (2026)", []).unwrap();
        conn.execute("INSERT INTO instructor (member_id, is_admin) VALUES (2, 1)", []).unwrap();
        conn.execute(
            "INSERT INTO enrollment (member_id, season_id, level) VALUES (1, 1, 'k')",
            [],
        )
        .unwrap();
    }

    fn first_skill_id(conn: &Connection, eval_type: &str) -> i64 {
        conn.query_row(
            "SELECT s.id FROM eval_skill s JOIN eval_skill_category c ON c.id = s.category_id \
             WHERE c.level = 'k' AND c.eval_type = ?1 AND s.parent_id IS NULL LIMIT 1",
            [eval_type],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn eval_catalog_seeded_and_idempotent() {
        let conn = mem_migrated();
        for lvl in ["k", "b", "A", "B"] {
            let c: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM eval_skill_category WHERE level = ?1",
                    [lvl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(c, 4, "level {lvl} should have 4 categories");
        }
        let skills: i64 = conn
            .query_row("SELECT COUNT(*) FROM eval_skill", [], |r| r.get(0))
            .unwrap();
        assert!(skills > 0);
        // Re-running migrations must not duplicate the catalog.
        schema::run_migrations(&conn).unwrap();
        let skills2: i64 = conn
            .query_row("SELECT COUNT(*) FROM eval_skill", [], |r| r.get(0))
            .unwrap();
        assert_eq!(skills, skills2);
    }

    #[test]
    fn save_and_get_progress_roundtrip() {
        let conn = mem_migrated();
        seed_student_and_instructor(&conn);
        let skill_id = first_skill_id(&conn, "standard");
        let p = SaveProgress {
            member_id: 1,
            season_id: 1,
            notes: "goed bezig".into(),
            days: vec![SaveDay {
                day: "Sat".into(),
                skippers: vec![2],
                records: BTreeMap::from([(skill_id.to_string(), "attempted".to_string())]),
            }],
        };
        save_student_progress(&conn, &p).unwrap();

        let got = get_season_progress(&conn, 1).unwrap();
        assert_eq!(got.len(), 1, "only the non-instructor student is listed");
        let sp = &got[0];
        assert_eq!(sp.member_id, 1);
        assert_eq!(sp.notes, "goed bezig");
        let day = sp.days.get("Sat").expect("Sat day present");
        assert_eq!(day.skippers, vec![2]);
        assert_eq!(day.records.get(&skill_id.to_string()).unwrap(), "attempted");

        // Save is replace-all: clearing days + notes empties the student.
        save_student_progress(
            &conn,
            &SaveProgress { member_id: 1, season_id: 1, notes: "".into(), days: vec![] },
        )
        .unwrap();
        let got = get_season_progress(&conn, 1).unwrap();
        assert!(got[0].days.is_empty() && got[0].notes.is_empty());
    }

    #[test]
    fn save_progress_validation_rejects_bad_input() {
        let conn = mem_migrated();
        seed_student_and_instructor(&conn);
        let std_skill = first_skill_id(&conn, "standard");
        let day = |skippers, records| {
            SaveProgress {
                member_id: 1,
                season_id: 1,
                notes: String::new(),
                days: vec![SaveDay { day: "Sat".into(), skippers, records }],
            }
        };
        let bad = |r: Result<(), DbError>| assert!(matches!(r, Err(DbError::Value(_))), "{r:?}");

        // invalid weekday
        bad(save_student_progress(
            &conn,
            &SaveProgress {
                member_id: 1,
                season_id: 1,
                notes: String::new(),
                days: vec![SaveDay { day: "Xyz".into(), skippers: vec![], records: BTreeMap::new() }],
            },
        ));
        // skipper that isn't an instructor (member 1 is the student)
        bad(save_student_progress(&conn, &day(vec![1], BTreeMap::new())));
        // more than two skippers
        bad(save_student_progress(&conn, &day(vec![2, 2, 2], BTreeMap::new())));
        // unknown skill id
        bad(save_student_progress(
            &conn,
            &day(vec![], BTreeMap::from([("999999".into(), "attempted".into())])),
        ));
        // wrong status for a standard skill (overall code)
        bad(save_student_progress(
            &conn,
            &day(vec![], BTreeMap::from([(std_skill.to_string(), "G".into())])),
        ));
        // member that isn't an enrolled student (the instructor)
        bad(save_student_progress(
            &conn,
            &SaveProgress { member_id: 2, season_id: 1, notes: String::new(), days: vec![] },
        ));

        // nothing was written despite the failed attempts
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM eval_assessment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
