//! SQLite connection management + schema. Mirrors `db.py`.
//!
//! Stage (a) scope: connection PRAGMAs, schema creation, and the idempotent
//! migrations. The member/season/schedule/instructor data helpers are added in
//! stage (b).

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
}
