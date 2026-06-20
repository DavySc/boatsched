//! Database schema + idempotent migrations.
//!
//! The `CREATE TABLE` DDL is reproduced **verbatim** from the Python
//! `db._create_tables()` (which is marked "do not modify"). The migration
//! functions mirror the Python idempotent ALTER/DROP helpers one-for-one and
//! run once at startup for pre-existing databases.

use rusqlite::Connection;

/// Schema DDL, copied verbatim from `db._create_tables()`.
pub const SCHEMA_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS member (
        id      INTEGER PRIMARY KEY,
        name    TEXT NOT NULL,
        active  BOOLEAN NOT NULL DEFAULT 1,
        joined  DATE NOT NULL DEFAULT (date('now'))
    );

    CREATE TABLE IF NOT EXISTS instructor (
        instructor_id INTEGER PRIMARY KEY,
        member_id     INTEGER NOT NULL UNIQUE REFERENCES member(id),
        password_hash BLOB,
        password_salt BLOB
    );

    CREATE TABLE IF NOT EXISTS season (
        id   INTEGER PRIMARY KEY,
        year INTEGER NOT NULL UNIQUE
    );

    CREATE TABLE IF NOT EXISTS schedule (
        id         INTEGER PRIMARY KEY,
        season_id  INTEGER NOT NULL REFERENCES season(id),
        created    TIMESTAMP DEFAULT (datetime('now')),
        start_date DATE,
        config     TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS boat (
        boat_id INTEGER PRIMARY KEY,
        name    TEXT,
        notes   TEXT
    );

    CREATE TABLE IF NOT EXISTS boat_assignment (
        id          INTEGER PRIMARY KEY,
        schedule_id INTEGER NOT NULL REFERENCES schedule(id),
        member_id   INTEGER NOT NULL REFERENCES member(id),
        boat_id     INTEGER NOT NULL REFERENCES boat(boat_id),
        day         TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS enrollment (
        member_id INTEGER NOT NULL REFERENCES member(id),
        season_id INTEGER NOT NULL REFERENCES season(id),
        level     TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
        PRIMARY KEY (member_id, season_id)
    );

    CREATE TABLE IF NOT EXISTS skill (
        id      INTEGER PRIMARY KEY,
        level   TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
        code    TEXT NOT NULL,
        label   TEXT NOT NULL,
        ordinal INTEGER NOT NULL,
        retired BOOLEAN NOT NULL DEFAULT 0,
        UNIQUE (level, code)
    );

    CREATE TABLE IF NOT EXISTS skill_result (
        member_id INTEGER NOT NULL,
        season_id INTEGER NOT NULL,
        skill_id  INTEGER NOT NULL REFERENCES skill(id),
        status    TEXT NOT NULL,
        note      TEXT,
        PRIMARY KEY (member_id, season_id, skill_id),
        FOREIGN KEY (member_id, season_id)
            REFERENCES enrollment(member_id, season_id)
    );

    CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_member_per_boat_assignment
        ON boat_assignment(schedule_id, member_id, day);

    CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_name ON member(name);

    CREATE INDEX IF NOT EXISTS ix_enrollment_season ON enrollment(season_id);

    CREATE VIEW IF NOT EXISTS v_level_card AS
    SELECT e.member_id, e.season_id, se.year AS season_year,
           m.name AS member_name, e.level,
           sk.id AS skill_id, sk.code AS skill_code, sk.label AS skill_label,
           sk.ordinal, sr.status, sr.note
    FROM enrollment e
    JOIN member m  ON m.id  = e.member_id
    JOIN season se ON se.id = e.season_id
    JOIN skill  sk ON sk.level = e.level AND sk.retired = 0
    LEFT JOIN skill_result sr
           ON sr.member_id = e.member_id
          AND sr.season_id = e.season_id
          AND sr.skill_id  = sk.id;
"#;

/// Create all tables/indexes/view and seed boats 1..=13. Mirrors
/// `_create_tables()`.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    for i in 1..=13 {
        conn.execute("INSERT OR IGNORE INTO boat (boat_id) VALUES (?1)", [i])?;
    }
    Ok(())
}

/// True when `ALTER TABLE … ADD COLUMN` failed because the column already
/// exists (the only OperationalError the Python code swallows).
fn ignore_dup_column(res: rusqlite::Result<usize>) -> rusqlite::Result<()> {
    match res {
        Ok(_) => Ok(()),
        // SQLite: "duplicate column name: …"
        Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Add `is_admin BOOLEAN NOT NULL DEFAULT 0` to instructor (idempotent).
pub fn add_admin_column(conn: &Connection) -> rusqlite::Result<()> {
    ignore_dup_column(conn.execute(
        "ALTER TABLE instructor ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT 0",
        [],
    ))
}

/// If no admin exists yet, promote all current instructors to admin.
pub fn auto_grant_first_admin(conn: &Connection) -> rusqlite::Result<()> {
    let has_admin: bool = conn
        .query_row(
            "SELECT 1 FROM instructor WHERE is_admin=1 LIMIT 1",
            [],
            |_| Ok(()),
        )
        .is_ok();
    if has_admin {
        return Ok(());
    }
    conn.execute("UPDATE instructor SET is_admin=1", [])?;
    Ok(())
}

/// Add `yjs_state BLOB` to schedule (idempotent).
pub fn add_yjs_state_column(conn: &Connection) -> rusqlite::Result<()> {
    ignore_dup_column(conn.execute("ALTER TABLE schedule ADD COLUMN yjs_state BLOB", []))
}

/// Add `start_date DATE` to schedule (idempotent).
pub fn add_start_date_column(conn: &Connection) -> rusqlite::Result<()> {
    ignore_dup_column(conn.execute("ALTER TABLE schedule ADD COLUMN start_date DATE", []))
}

/// Create enrollment/skill/skill_result + index + view (idempotent). The
/// `IF NOT EXISTS` DDL is a subset of [`SCHEMA_SQL`]; reproduced verbatim from
/// `add_enrollment_tables`.
pub fn add_enrollment_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS enrollment (
            member_id INTEGER NOT NULL REFERENCES member(id),
            season_id INTEGER NOT NULL REFERENCES season(id),
            level     TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            PRIMARY KEY (member_id, season_id)
        );

        CREATE TABLE IF NOT EXISTS skill (
            id      INTEGER PRIMARY KEY,
            level   TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            code    TEXT NOT NULL,
            label   TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            retired BOOLEAN NOT NULL DEFAULT 0,
            UNIQUE (level, code)
        );

        CREATE TABLE IF NOT EXISTS skill_result (
            member_id INTEGER NOT NULL,
            season_id INTEGER NOT NULL,
            skill_id  INTEGER NOT NULL REFERENCES skill(id),
            status    TEXT NOT NULL,
            note      TEXT,
            PRIMARY KEY (member_id, season_id, skill_id),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );

        CREATE INDEX IF NOT EXISTS ix_enrollment_season ON enrollment(season_id);

        CREATE VIEW IF NOT EXISTS v_level_card AS
        SELECT e.member_id, e.season_id, se.year AS season_year,
               m.name AS member_name, e.level,
               sk.id AS skill_id, sk.code AS skill_code, sk.label AS skill_label,
               sk.ordinal, sr.status, sr.note
        FROM enrollment e
        JOIN member m  ON m.id  = e.member_id
        JOIN season se ON se.id = e.season_id
        JOIN skill  sk ON sk.level = e.level AND sk.retired = 0
        LEFT JOIN skill_result sr
               ON sr.member_id = e.member_id
              AND sr.season_id = e.season_id
              AND sr.skill_id  = sk.id;
        "#,
    )
}

/// Create the evaluation tables and seed the skill catalog (idempotent).
///
/// These are *new* tables, not part of the frozen `_create_tables` DDL. They
/// supersede the unused, never-seeded `skill`/`skill_result`/`v_level_card`
/// schema (left in place) with a model that matches the eval card: a
/// `(student × day × skill)` grid, per-day skipper instructor(s), and per-student
/// notes. Namespaced `eval_*`. The catalog is seeded once via
/// [`crate::eval_catalog::seed_catalog`].
pub fn add_eval_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS eval_skill_category (
            id        INTEGER PRIMARY KEY,
            level     TEXT NOT NULL CHECK (level IN ('k','b','A','B')),
            name      TEXT NOT NULL,
            eval_type TEXT NOT NULL CHECK (eval_type IN ('standard','overall')),
            ordinal   INTEGER NOT NULL,
            UNIQUE (level, name)
        );

        CREATE TABLE IF NOT EXISTS eval_skill (
            id          INTEGER PRIMARY KEY,
            category_id INTEGER NOT NULL REFERENCES eval_skill_category(id),
            parent_id   INTEGER REFERENCES eval_skill(id),
            label       TEXT NOT NULL,
            ordinal     INTEGER NOT NULL,
            retired     INTEGER NOT NULL DEFAULT 0,
            UNIQUE (category_id, parent_id, label)
        );

        CREATE TABLE IF NOT EXISTS eval_assessment (
            member_id INTEGER NOT NULL,
            season_id INTEGER NOT NULL,
            skill_id  INTEGER NOT NULL REFERENCES eval_skill(id),
            day       TEXT NOT NULL CHECK (day IN ('Fri','Sat','Sun','Mon','Tue','Wed')),
            status    TEXT NOT NULL CHECK (status IN ('attempted','completed','O','T','V','G')),
            PRIMARY KEY (member_id, season_id, skill_id, day),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );

        CREATE INDEX IF NOT EXISTS ix_eval_assessment_student
            ON eval_assessment(member_id, season_id);

        CREATE TABLE IF NOT EXISTS eval_skipper (
            member_id     INTEGER NOT NULL,
            season_id     INTEGER NOT NULL,
            day           TEXT NOT NULL CHECK (day IN ('Fri','Sat','Sun','Mon','Tue','Wed')),
            position      INTEGER NOT NULL CHECK (position IN (1,2)),
            instructor_id INTEGER NOT NULL REFERENCES instructor(member_id),
            PRIMARY KEY (member_id, season_id, day, position),
            UNIQUE (member_id, season_id, day, instructor_id),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );

        CREATE TABLE IF NOT EXISTS eval_note (
            member_id INTEGER NOT NULL,
            season_id INTEGER NOT NULL,
            note      TEXT NOT NULL,
            PRIMARY KEY (member_id, season_id),
            FOREIGN KEY (member_id, season_id)
                REFERENCES enrollment(member_id, season_id)
        );
        "#,
    )?;
    crate::eval_catalog::seed_catalog(conn)
}

/// Remove the legacy `member.level` column if present. SQLite ≥ 3.35 supports
/// DROP COLUMN; rusqlite's bundled SQLite is recent, so the fallback no-op
/// branch from Python is unnecessary but harmless to keep.
pub fn drop_member_level_column(conn: &Connection) -> rusqlite::Result<()> {
    let has_level: bool = conn
        .prepare("PRAGMA table_info(member)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "level");
    if has_level {
        conn.execute("ALTER TABLE member DROP COLUMN level", [])?;
    }
    Ok(())
}

/// Drop the legacy `progress_record` / `day_allocation` tables (idempotent).
pub fn drop_legacy_camp_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS progress_record; DROP TABLE IF EXISTS day_allocation;")
}

/// Error raised when member names collide only by case and must be merged
/// before the unique name index can be made case-insensitive.
#[derive(Debug, thiserror::Error)]
#[error("cannot make member names case-insensitive: existing names collide only by case and must be merged manually first: {0}")]
pub struct NameCollisionError(pub String);

/// Rebuild `ix_unique_name` as a `COLLATE NOCASE` unique index (idempotent).
/// Mirrors `make_name_index_nocase`, including the collision guard.
pub fn make_name_index_nocase(conn: &Connection) -> Result<(), MigrationError> {
    let existing_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='ix_unique_name'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional_flatten();

    if let Some(sql) = &existing_sql {
        if sql.to_uppercase().contains("NOCASE") {
            return Ok(());
        }
    }

    let collisions: Vec<String> = conn
        .prepare(
            "SELECT group_concat(name, ' | ') AS names FROM member \
             GROUP BY name COLLATE NOCASE HAVING COUNT(*) > 1",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect();
    if !collisions.is_empty() {
        return Err(MigrationError::NameCollision(NameCollisionError(
            collisions.join("; "),
        )));
    }

    conn.execute_batch(
        "DROP INDEX IF EXISTS ix_unique_name; \
         CREATE UNIQUE INDEX ix_unique_name ON member(name COLLATE NOCASE);",
    )?;
    Ok(())
}

/// Errors from migrations that can fail with domain meaning.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    NameCollision(#[from] NameCollisionError),
}

/// Small helper: flatten `Result<Option<T>>` from an optional single-row query
/// into `Option<T>`, treating a missing row as `None`.
trait OptionalFlatten<T> {
    fn optional_flatten(self) -> Option<T>;
}
impl<T> OptionalFlatten<T> for rusqlite::Result<Option<T>> {
    fn optional_flatten(self) -> Option<T> {
        match self {
            Ok(opt) => opt,
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

/// Run every migration once, in the same order as `create_app()`.
pub fn run_migrations(conn: &Connection) -> Result<(), MigrationError> {
    add_yjs_state_column(conn)?;
    add_start_date_column(conn)?;
    add_enrollment_tables(conn)?;
    drop_member_level_column(conn)?;
    drop_legacy_camp_tables(conn)?;
    add_admin_column(conn)?;
    auto_grant_first_admin(conn)?;
    make_name_index_nocase(conn)?;
    add_eval_tables(conn)?;
    Ok(())
}
