//! Data-access helpers — a one-for-one port of the functions in `db.py`.
//!
//! SQL strings, `COLLATE NOCASE` matching, `ORDER BY`, and `ON CONFLICT`
//! clauses are reproduced verbatim. Error variants distinguish the Python
//! `RuntimeError` cases (which the server maps to 400/500) from `ValueError`
//! cases (mapped to 400/409); the exact (Dutch) messages are preserved so the
//! server's response bodies match.

use boatsched_core::{Kid, ScheduleRow};
use rusqlite::{params, Connection, OptionalExtension};

/// Errors from the data layer. Variants mirror the two Python exception types
/// the server treats differently.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// Mirrors a Python `RuntimeError` (e.g. missing member / enrollment).
    #[error("{0}")]
    Runtime(String),
    /// Mirrors a Python `ValueError` (e.g. empty/colliding name, instructor).
    #[error("{0}")]
    Value(String),
}

type Result<T> = std::result::Result<T, DbError>;

// ---------------------------------------------------------------------------
// Row types (named structs replace the Python dict/tuple returns)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Member {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Season {
    pub id: i64,
    pub year: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScheduleMeta {
    pub id: i64,
    pub created: Option<String>,
    pub start_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Boat {
    pub id: i64,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstructorRow {
    pub id: i64,
    pub name: String,
    /// Kept as integer to match the JSON the Python endpoint emits (tests
    /// compare `== 1` / `== 0`).
    pub is_admin: i64,
    pub has_password: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeedResult {
    pub created: i64,
    pub existing: i64,
    pub total: i64,
}

// ---------------------------------------------------------------------------
// Member / enrollment helpers
// ---------------------------------------------------------------------------

/// Upsert members from a roster; if `season_id` given, also upsert enrollments.
/// `'k'`-level rows are auto-created; every other level must already exist
/// (NOCASE) or a [`DbError::Runtime`] is returned. Mirrors
/// `add_or_update_members`.
pub fn add_or_update_members(
    conn: &Connection,
    members: &[Kid],
    season_id: Option<i64>,
) -> Result<()> {
    // Auto-create members for k-level (new participants this season).
    for kid in members.iter().filter(|k| k.level == "k") {
        conn.execute(
            "INSERT INTO member (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![kid.name],
        )?;
    }

    // Resolve member IDs; non-k members must already be in the table.
    let mut id_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut missing: Vec<String> = Vec::new();
    for kid in members {
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM member WHERE name = ?1 COLLATE NOCASE",
                params![kid.name],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => {
                id_map.insert(kid.name.clone(), id);
            }
            None => missing.push(format!("'{}' (level '{}')", kid.name, kid.level)),
        }
    }
    if !missing.is_empty() {
        return Err(DbError::Runtime(format!(
            "returning members not found in member table: {}; only 'k'-level entries are auto-created",
            missing.join(", ")
        )));
    }

    if let Some(sid) = season_id {
        for kid in members {
            conn.execute(
                "INSERT INTO enrollment (member_id, season_id, level) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(member_id, season_id) DO UPDATE SET level = excluded.level",
                params![id_map[&kid.name], sid, kid.level],
            )?;
        }
    }
    Ok(())
}

/// `INSERT OR IGNORE` a list of names. Trims, drops blank/None. Mirrors
/// `seed_members`.
pub fn seed_members(conn: &Connection, names: &[&str]) -> Result<SeedResult> {
    let clean: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let count_all = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT COUNT(*) FROM member", [], |r| r.get(0))?)
    };
    if clean.is_empty() {
        let total = count_all(conn)?;
        return Ok(SeedResult {
            created: 0,
            existing: 0,
            total,
        });
    }
    let before = count_all(conn)?;
    for n in &clean {
        conn.execute(
            "INSERT OR IGNORE INTO member (name) VALUES (?1)",
            params![n],
        )?;
    }
    let after = count_all(conn)?;
    let created = after - before;
    Ok(SeedResult {
        created,
        existing: clean.len() as i64 - created,
        total: after,
    })
}

/// All active members ordered by name. Mirrors `list_members`.
pub fn list_members(conn: &Connection) -> Result<Vec<Member>> {
    let mut stmt = conn.prepare("SELECT id, name FROM member WHERE active=1 ORDER BY name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Member {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Soft-delete a member. [`DbError::Value`] (Dutch) if they are an instructor.
/// Mirrors `delete_member`.
pub fn delete_member(conn: &Connection, member_id: i64) -> Result<()> {
    let is_inst: bool = conn
        .query_row(
            "SELECT 1 FROM instructor WHERE member_id=?1",
            params![member_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if is_inst {
        return Err(DbError::Value(
            "lid is een instructeur — verwijder eerst de instructeursrol".into(),
        ));
    }
    conn.execute("UPDATE member SET active=0 WHERE id=?1", params![member_id])?;
    Ok(())
}

/// Rename a member. [`DbError::Value`] on empty name or collision. Mirrors
/// `rename_member`.
pub fn rename_member(conn: &Connection, member_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::Value("name must not be empty".into()));
    }
    match conn.execute(
        "UPDATE member SET name=?1 WHERE id=?2",
        params![name, member_id],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(DbError::Value(format!("naam '{name}' bestaat al")))
        }
        Err(e) => Err(e.into()),
    }
}

/// Upsert a single enrollment row. Mirrors `add_enrollment`.
pub fn add_enrollment(
    conn: &Connection,
    member_id: i64,
    season_id: i64,
    level: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO enrollment (member_id, season_id, level) VALUES (?1, ?2, ?3) \
         ON CONFLICT(member_id, season_id) DO UPDATE SET level = excluded.level",
        params![member_id, season_id, level],
    )?;
    Ok(())
}

/// `[(name, level)]` for active members enrolled in `season_id`, ordered by
/// name. Mirrors `get_season_roster`.
pub fn get_season_roster(conn: &Connection, season_id: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT m.name, e.level FROM enrollment e \
         JOIN member m ON m.id = e.member_id \
         WHERE e.season_id = ?1 AND m.active = 1 ORDER BY m.name",
    )?;
    let rows = stmt
        .query_map(params![season_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Update the level of an *existing* enrollment row only. Returns whether a
/// row was updated. Mirrors `set_enrollment_level`.
pub fn set_enrollment_level(
    conn: &Connection,
    member_id: i64,
    season_id: i64,
    level: &str,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE enrollment SET level = ?1 WHERE member_id = ?2 AND season_id = ?3",
        params![level, member_id, season_id],
    )?;
    Ok(n > 0)
}

/// `[(name, level)]` for non-instructor members enrolled in `season_id`.
/// Mirrors `list_enrolled_students`.
pub fn list_enrolled_students(conn: &Connection, season_id: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT m.name, e.level FROM enrollment e \
         JOIN member m ON m.id = e.member_id \
         WHERE e.season_id = ?1 \
           AND m.id NOT IN (SELECT member_id FROM instructor) \
         ORDER BY m.name",
    )?;
    let rows = stmt
        .query_map(params![season_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// True when `member_id` is enrolled in `season_id` and is not an instructor.
/// Mirrors `is_student`.
pub fn is_student(conn: &Connection, member_id: i64, season_id: i64) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM enrollment e WHERE e.member_id = ?1 AND e.season_id = ?2 \
               AND e.member_id NOT IN (SELECT member_id FROM instructor)",
            params![member_id, season_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(found)
}

// ---------------------------------------------------------------------------
// Season / schedule helpers
// ---------------------------------------------------------------------------

/// Create a season, return its id. Mirrors `create_season`.
pub fn create_season(conn: &Connection, year: i64) -> Result<i64> {
    conn.execute("INSERT INTO season (year) VALUES (?1)", params![year])?;
    Ok(conn.last_insert_rowid())
}

/// Seasons ordered by year DESC. Mirrors `list_seasons`.
pub fn list_seasons(conn: &Connection) -> Result<Vec<Season>> {
    let mut stmt = conn.prepare("SELECT id, year FROM season ORDER BY year DESC")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Season {
                id: r.get(0)?,
                year: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Persist a schedule + its boat assignments. Looks members up by NOCASE name;
/// unknown name ⇒ [`DbError::Runtime`]. Atomic (rolls back on error). Mirrors
/// `save_schedule`.
pub fn save_schedule(
    conn: &Connection,
    season_id: i64,
    config_json: &str,
    result: &[ScheduleRow],
) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO schedule (season_id, config) VALUES (?1, ?2)",
        params![season_id, config_json],
    )?;
    let schedule_id = tx.last_insert_rowid();
    for row in result {
        let member_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM member WHERE name = ?1 COLLATE NOCASE",
                params![row.name],
                |r| r.get(0),
            )
            .optional()?;
        let member_id = member_id
            .ok_or_else(|| DbError::Runtime(format!("member '{}' not found in DB", row.name)))?;
        tx.execute(
            "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_id, member_id, row.boat, row.day],
        )?;
    }
    tx.commit()?;
    Ok(schedule_id)
}

/// Schedules for a season, newest first. Mirrors `list_schedules`.
pub fn list_schedules(conn: &Connection, season_id: i64) -> Result<Vec<ScheduleMeta>> {
    let mut stmt = conn.prepare(
        "SELECT id, created, start_date FROM schedule WHERE season_id=?1 ORDER BY id DESC",
    )?;
    let rows = stmt
        .query_map(params![season_id], |r| {
            Ok(ScheduleMeta {
                id: r.get(0)?,
                created: r.get(1)?,
                start_date: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Schedule rows; [`DbError::Runtime`] if any member lacks an enrollment in the
/// schedule's season. Unknown id ⇒ empty. Ordered by day, boat, name. Mirrors
/// `get_schedule`.
pub fn get_schedule(conn: &Connection, schedule_id: i64) -> Result<Vec<ScheduleRow>> {
    let mut stmt = conn.prepare(
        "SELECT ba.day, ba.boat_id AS boat, m.name, e.level \
         FROM boat_assignment ba \
         JOIN member m ON m.id = ba.member_id \
         JOIN schedule s ON s.id = ba.schedule_id \
         LEFT JOIN enrollment e ON e.member_id = m.id AND e.season_id = s.season_id \
         WHERE ba.schedule_id = ?1 \
         ORDER BY ba.day, ba.boat_id, m.name",
    )?;
    let rows = stmt
        .query_map(params![schedule_id], |r| {
            let level: Option<String> = r.get(3)?;
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                level,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut missing: Vec<String> = rows
        .iter()
        .filter(|(_, _, _, lv)| lv.is_none())
        .map(|(_, _, name, _)| name.clone())
        .collect();
    if !missing.is_empty() {
        let season_id: Option<i64> = conn
            .query_row(
                "SELECT season_id FROM schedule WHERE id=?1",
                params![schedule_id],
                |r| r.get(0),
            )
            .optional()?;
        missing.sort();
        missing.dedup();
        let season = season_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        return Err(DbError::Runtime(format!(
            "members not enrolled in season {season}: {}",
            missing.join(", ")
        )));
    }

    Ok(rows
        .into_iter()
        .map(|(day, boat, name, level)| ScheduleRow {
            day,
            boat,
            name,
            level: level.unwrap_or_default(),
        })
        .collect())
}

/// Swap two members' boats on a given day. [`DbError::Runtime`] if either is
/// not in the schedule that day; same boat ⇒ no-op. Mirrors `swap_assignment`.
pub fn swap_assignment(
    conn: &Connection,
    schedule_id: i64,
    day: &str,
    name_a: &str,
    name_b: &str,
) -> Result<()> {
    let row_of = |name: &str| -> Result<Option<(i64, i64)>> {
        Ok(conn
            .query_row(
                "SELECT ba.id, ba.boat_id FROM boat_assignment ba \
                 JOIN member m ON m.id = ba.member_id \
                 WHERE ba.schedule_id=?1 AND ba.day=?2 AND m.name=?3 COLLATE NOCASE",
                params![schedule_id, day, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    };
    let (row_a, row_b) = (row_of(name_a)?, row_of(name_b)?);
    let (row_a, row_b) = match (row_a, row_b) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Err(DbError::Runtime(
                "member not found in schedule for that day".into(),
            ))
        }
    };
    if row_a.1 == row_b.1 {
        return Ok(());
    }
    conn.execute(
        "UPDATE boat_assignment SET boat_id=?1 WHERE id=?2",
        params![row_b.1, row_a.0],
    )?;
    conn.execute(
        "UPDATE boat_assignment SET boat_id=?1 WHERE id=?2",
        params![row_a.1, row_b.0],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Boat helpers
// ---------------------------------------------------------------------------

/// Boats ordered by id. Mirrors `list_boats`.
pub fn list_boats(conn: &Connection) -> Result<Vec<Boat>> {
    let mut stmt = conn.prepare("SELECT boat_id AS id, name FROM boat ORDER BY boat_id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Boat {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Rename a boat; empty/None ⇒ stored NULL. Mirrors `rename_boat`.
pub fn rename_boat(conn: &Connection, boat_id: i64, name: Option<&str>) -> Result<()> {
    let value: Option<&str> = match name {
        Some(s) if !s.is_empty() => Some(s),
        _ => None,
    };
    conn.execute(
        "UPDATE boat SET name=?1 WHERE boat_id=?2",
        params![value, boat_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Instructor / auth helpers
// ---------------------------------------------------------------------------

/// scrypt parameters identical to the Python original (`n=2**14, r=8, p=1`,
/// 64-byte derived key, 32-byte salt).
fn scrypt_hash(password: &[u8], salt: &[u8]) -> [u8; 64] {
    let params = scrypt::Params::new(14, 8, 1, 64).expect("valid scrypt params");
    let mut out = [0u8; 64];
    scrypt::scrypt(password, salt, &params, &mut out).expect("scrypt output len");
    out
}

/// Set (or replace) an instructor's password. Mirrors `set_instructor_password`.
pub fn set_instructor_password(conn: &Connection, member_id: i64, password: &str) -> Result<()> {
    use rand::RngCore;
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    let dk = scrypt_hash(password.as_bytes(), &salt);
    conn.execute(
        "UPDATE instructor SET password_hash=?1, password_salt=?2 WHERE member_id=?3",
        params![dk.as_slice(), salt.as_slice(), member_id],
    )?;
    Ok(())
}

/// Verify a login by NOCASE name. Returns the member id on success. Mirrors
/// `verify_instructor_login` (constant-time compare).
pub fn verify_instructor_login(
    conn: &Connection,
    name: &str,
    password: &str,
) -> Result<Option<i64>> {
    let row: Option<(i64, Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT i.member_id, i.password_hash, i.password_salt \
             FROM instructor i JOIN member m ON m.id = i.member_id \
             WHERE m.name=?1 COLLATE NOCASE \
             AND i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (member_id, expected, salt) = match row {
        Some(t) => t,
        None => return Ok(None),
    };
    let dk = scrypt_hash(password.as_bytes(), &salt);
    use subtle::ConstantTimeEq;
    if dk.as_slice().ct_eq(expected.as_slice()).into() {
        Ok(Some(member_id))
    } else {
        Ok(None)
    }
}

/// List instructors with admin/password flags. Mirrors `list_instructors`.
pub fn list_instructors(conn: &Connection) -> Result<Vec<InstructorRow>> {
    let mut stmt = conn.prepare(
        "SELECT i.member_id AS id, m.name, i.is_admin, \
         (i.password_hash IS NOT NULL AND i.password_salt IS NOT NULL) AS has_password \
         FROM instructor i JOIN member m ON m.id = i.member_id ORDER BY m.name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(InstructorRow {
                id: r.get(0)?,
                name: r.get(1)?,
                is_admin: r.get(2)?,
                has_password: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Set the is_admin flag. Mirrors `set_admin`.
pub fn set_admin(conn: &Connection, member_id: i64, is_admin: bool) -> Result<()> {
    conn.execute(
        "UPDATE instructor SET is_admin=?1 WHERE member_id=?2",
        params![if is_admin { 1 } else { 0 }, member_id],
    )?;
    Ok(())
}

/// Seed a bootstrap admin (idempotent). Sets the password only if none exists.
/// Returns true on first seed. Mirrors `ensure_admin`.
pub fn ensure_admin(conn: &Connection, name: &str, password: &str) -> Result<bool> {
    let name = name.trim();
    if name.is_empty() || password.is_empty() {
        return Err(DbError::Value(
            "admin name and password are required".into(),
        ));
    }
    let member_id: i64 = match conn
        .query_row(
            "SELECT id FROM member WHERE name=?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(id) => id,
        None => {
            conn.execute("INSERT INTO member (name) VALUES (?1)", params![name])?;
            conn.last_insert_rowid()
        }
    };
    let existing_hash: Option<Option<Vec<u8>>> = conn
        .query_row(
            "SELECT password_hash FROM instructor WHERE member_id=?1",
            params![member_id],
            |r| r.get(0),
        )
        .optional()?;
    match &existing_hash {
        None => {
            conn.execute(
                "INSERT INTO instructor (member_id, is_admin) VALUES (?1, 1)",
                params![member_id],
            )?;
        }
        Some(_) => {
            conn.execute(
                "UPDATE instructor SET is_admin=1 WHERE member_id=?1",
                params![member_id],
            )?;
        }
    }
    // If the instructor already had a password, keep it.
    if let Some(Some(_hash)) = existing_hash {
        return Ok(false);
    }
    set_instructor_password(conn, member_id, password)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Yjs / schedule state helpers
// ---------------------------------------------------------------------------

/// Mirrors `get_yjs_state`.
pub fn get_yjs_state(conn: &Connection, schedule_id: i64) -> Result<Option<Vec<u8>>> {
    let state: Option<Option<Vec<u8>>> = conn
        .query_row(
            "SELECT yjs_state FROM schedule WHERE id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(state.flatten())
}

/// Mirrors `set_yjs_state`.
pub fn set_yjs_state(conn: &Connection, schedule_id: i64, state: &[u8]) -> Result<()> {
    conn.execute(
        "UPDATE schedule SET yjs_state=?1 WHERE id=?2",
        params![state, schedule_id],
    )?;
    Ok(())
}

/// Mirrors `get_schedule_start_date`.
pub fn get_schedule_start_date(conn: &Connection, schedule_id: i64) -> Result<Option<String>> {
    let v: Option<Option<String>> = conn
        .query_row(
            "SELECT start_date FROM schedule WHERE id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.flatten())
}

/// Mirrors `set_schedule_start_date`.
pub fn set_schedule_start_date(
    conn: &Connection,
    schedule_id: i64,
    date_str: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE schedule SET start_date=?1 WHERE id=?2",
        params![date_str, schedule_id],
    )?;
    Ok(())
}

/// Replace all boat assignments for a schedule from a Yjs commit. Atomic;
/// unknown name ⇒ rollback + [`DbError::Runtime`]. `assignments` are
/// `(name, day, boat_id)`. Mirrors `replace_assignments`.
pub fn replace_assignments(
    conn: &Connection,
    schedule_id: i64,
    assignments: &[(String, String, i64)],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM boat_assignment WHERE schedule_id=?1",
        params![schedule_id],
    )?;
    for (name, day, boat_id) in assignments {
        let member_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM member WHERE name=?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        let member_id =
            member_id.ok_or_else(|| DbError::Runtime(format!("member '{name}' not found")))?;
        tx.execute(
            "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_id, member_id, boat_id, day],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Skill catalog + student progress (evaluation cards)
// ---------------------------------------------------------------------------

use crate::eval_catalog::allowed_statuses;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A sub-skill in the catalog.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogSubskill {
    pub id: i64,
    pub label: String,
}

/// A top-level skill, with its sub-skills.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogSkillRow {
    pub id: i64,
    pub label: String,
    pub subskills: Vec<CatalogSubskill>,
}

/// A category, carrying its `eval_type` and ordered skills.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogCategoryRow {
    pub level: String,
    pub category: String,
    pub eval_type: String,
    pub skills: Vec<CatalogSkillRow>,
}

/// The whole seeded catalog, ordered by category/skill/sub-skill ordinal. The
/// server groups this by `level` for `GET /api/skills`.
pub fn list_skill_catalog(conn: &Connection) -> Result<Vec<CatalogCategoryRow>> {
    let mut cstmt = conn.prepare(
        "SELECT id, level, name, eval_type FROM eval_skill_category ORDER BY level, ordinal",
    )?;
    let cats = cstmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(cats.len());
    for (cid, level, name, eval_type) in cats {
        let mut sstmt = conn.prepare(
            "SELECT id, label FROM eval_skill \
             WHERE category_id = ?1 AND parent_id IS NULL AND retired = 0 ORDER BY ordinal",
        )?;
        let top = sstmt
            .query_map(params![cid], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut skills = Vec::with_capacity(top.len());
        for (sid, label) in top {
            let mut substmt = conn.prepare(
                "SELECT id, label FROM eval_skill \
                 WHERE parent_id = ?1 AND retired = 0 ORDER BY ordinal",
            )?;
            let subskills = substmt
                .query_map(params![sid], |r| {
                    Ok(CatalogSubskill {
                        id: r.get(0)?,
                        label: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            skills.push(CatalogSkillRow {
                id: sid,
                label,
                subskills,
            });
        }
        out.push(CatalogCategoryRow {
            level,
            category: name,
            eval_type,
            skills,
        });
    }
    Ok(out)
}

/// One day's progress for a student: ordered skipper instructor ids + the
/// `skill_id → status` cells set on that day.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DayProgress {
    pub skippers: Vec<i64>,
    /// Keyed by `skill_id` (as a string, to be a JSON object key).
    pub records: BTreeMap<String, String>,
}

/// A student's full progress for a season.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StudentProgress {
    pub member_id: i64,
    pub name: String,
    pub level: String,
    pub notes: String,
    /// Keyed by weekday code (`"Sat"`, …).
    pub days: BTreeMap<String, DayProgress>,
}

/// All enrolled non-instructor students for `season_id` with their stored
/// assessments, skippers and notes, folded by day. Students with no data yet
/// are included (empty `days`/`notes`) so the frontend can key by name.
pub fn get_season_progress(conn: &Connection, season_id: i64) -> Result<Vec<StudentProgress>> {
    // Students (non-instructor enrollees).
    let mut sstmt = conn.prepare(
        "SELECT m.id, m.name, e.level FROM enrollment e \
         JOIN member m ON m.id = e.member_id \
         WHERE e.season_id = ?1 \
           AND m.id NOT IN (SELECT member_id FROM instructor) \
         ORDER BY m.name",
    )?;
    let students = sstmt
        .query_map(params![season_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Notes by member.
    let mut nstmt = conn.prepare("SELECT member_id, note FROM eval_note WHERE season_id = ?1")?;
    let notes: HashMap<i64, String> = nstmt
        .query_map(params![season_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;

    // member_id → day → DayProgress
    let mut by_member: HashMap<i64, BTreeMap<String, DayProgress>> = HashMap::new();

    let mut astmt = conn.prepare(
        "SELECT member_id, day, skill_id, status FROM eval_assessment WHERE season_id = ?1",
    )?;
    let arows = astmt.query_map(params![season_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in arows {
        let (mid, day, skill_id, status) = row?;
        by_member
            .entry(mid)
            .or_default()
            .entry(day)
            .or_default()
            .records
            .insert(skill_id.to_string(), status);
    }

    let mut kstmt = conn.prepare(
        "SELECT member_id, day, instructor_id FROM eval_skipper \
         WHERE season_id = ?1 ORDER BY position",
    )?;
    let krows = kstmt.query_map(params![season_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in krows {
        let (mid, day, instructor_id) = row?;
        by_member
            .entry(mid)
            .or_default()
            .entry(day)
            .or_default()
            .skippers
            .push(instructor_id);
    }

    let out = students
        .into_iter()
        .map(|(id, name, level)| StudentProgress {
            member_id: id,
            name,
            level,
            notes: notes.get(&id).cloned().unwrap_or_default(),
            days: by_member.remove(&id).unwrap_or_default(),
        })
        .collect();
    Ok(out)
}

/// One day of an incoming save.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SaveDay {
    pub day: String,
    #[serde(default)]
    pub skippers: Vec<i64>,
    /// Keyed by `skill_id` (string); empty status values are ignored.
    #[serde(default)]
    pub records: BTreeMap<String, String>,
}

/// The full payload for `POST /api/progress`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SaveProgress {
    pub member_id: i64,
    pub season_id: i64,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub days: Vec<SaveDay>,
}

const WEEKDAYS: [&str; 6] = ["Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// Validate and persist a student's whole eval card for a season in one
/// transaction (replace-all for assessments + skippers, upsert/clear notes).
///
/// Validation (all → [`DbError::Value`]): member is an enrolled non-instructor
/// student; every `day` is a weekday code; each day has ≤2 distinct skipper
/// instructors that are actually instructors; every `skill_id` belongs to the
/// student's enrolled level and each status is legal for that skill's category
/// `eval_type`.
pub fn save_student_progress(conn: &Connection, p: &SaveProgress) -> Result<()> {
    if !is_student(conn, p.member_id, p.season_id)? {
        return Err(DbError::Value(
            "geen ingeschreven deelnemer voor dit seizoen".into(),
        ));
    }
    let level: String = conn.query_row(
        "SELECT level FROM enrollment WHERE member_id = ?1 AND season_id = ?2",
        params![p.member_id, p.season_id],
        |r| r.get(0),
    )?;

    // skill_id → eval_type for the student's level (top-level skills + sub-skills).
    let mut skstmt = conn.prepare(
        "SELECT s.id, c.eval_type FROM eval_skill s \
         JOIN eval_skill_category c ON c.id = s.category_id \
         WHERE c.level = ?1 AND s.retired = 0",
    )?;
    let level_skills: HashMap<i64, String> = skstmt
        .query_map(params![level], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;

    // Validate every day before writing anything.
    for d in &p.days {
        if !WEEKDAYS.contains(&d.day.as_str()) {
            return Err(DbError::Value(format!("ongeldige dag: {}", d.day)));
        }
        if d.skippers.len() > 2 {
            return Err(DbError::Value("maximaal twee schippers per dag".into()));
        }
        let mut seen = HashSet::new();
        for &iid in &d.skippers {
            if !seen.insert(iid) {
                return Err(DbError::Value("dubbele schipper".into()));
            }
            let is_instr = conn
                .query_row(
                    "SELECT 1 FROM instructor WHERE member_id = ?1",
                    params![iid],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !is_instr {
                return Err(DbError::Value(format!(
                    "schipper {iid} is geen instructeur"
                )));
            }
        }
        for (sid_str, status) in &d.records {
            if status.is_empty() {
                continue;
            }
            let sid: i64 = sid_str
                .parse()
                .map_err(|_| DbError::Value(format!("ongeldige skill id: {sid_str}")))?;
            let eval_type = level_skills.get(&sid).ok_or_else(|| {
                DbError::Value(format!("skill {sid} hoort niet bij niveau {level}"))
            })?;
            if !allowed_statuses(eval_type).contains(&status.as_str()) {
                return Err(DbError::Value(format!(
                    "ongeldige status '{status}' voor skill {sid}"
                )));
            }
        }
    }

    // Persist: replace this student's rows for the season atomically.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM eval_assessment WHERE member_id = ?1 AND season_id = ?2",
        params![p.member_id, p.season_id],
    )?;
    tx.execute(
        "DELETE FROM eval_skipper WHERE member_id = ?1 AND season_id = ?2",
        params![p.member_id, p.season_id],
    )?;

    let trimmed = p.notes.trim();
    if trimmed.is_empty() {
        tx.execute(
            "DELETE FROM eval_note WHERE member_id = ?1 AND season_id = ?2",
            params![p.member_id, p.season_id],
        )?;
    } else {
        tx.execute(
            "INSERT INTO eval_note (member_id, season_id, note) VALUES (?1, ?2, ?3) \
             ON CONFLICT(member_id, season_id) DO UPDATE SET note = excluded.note",
            params![p.member_id, p.season_id, trimmed],
        )?;
    }

    for d in &p.days {
        for (pos, &iid) in d.skippers.iter().enumerate() {
            tx.execute(
                "INSERT INTO eval_skipper (member_id, season_id, day, position, instructor_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.member_id, p.season_id, d.day, (pos as i64) + 1, iid],
            )?;
        }
        for (sid_str, status) in &d.records {
            if status.is_empty() {
                continue;
            }
            let sid: i64 = sid_str.parse().expect("validated above");
            tx.execute(
                "INSERT INTO eval_assessment (member_id, season_id, skill_id, day, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.member_id, p.season_id, sid, d.day, status],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}
