use super::{DbError, Result};
use boatsched_core::Kid;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Member {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeedResult {
    pub created: i64,
    pub existing: i64,
    pub total: i64,
}

/// Upsert members from a roster; if `season_id` given, also upsert enrollments.
/// `'k'`-level rows are auto-created; every other level must already exist
/// (NOCASE) or a [`DbError::Missing`] is returned. Mirrors
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
        return Err(DbError::Missing(format!(
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

/// Soft-delete a member. [`DbError::Invalid`] (Dutch) if they are an instructor.
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
        return Err(DbError::Invalid(
            "lid is een instructeur — verwijder eerst de instructeursrol".into(),
        ));
    }
    conn.execute("UPDATE member SET active=0 WHERE id=?1", params![member_id])?;
    Ok(())
}

/// Rename a member. [`DbError::Invalid`] on empty name or collision. Mirrors
/// `rename_member`.
pub fn rename_member(conn: &Connection, member_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::Invalid("name must not be empty".into()));
    }
    match conn.execute(
        "UPDATE member SET name=?1 WHERE id=?2",
        params![name, member_id],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(DbError::Invalid(format!("naam '{name}' bestaat al")))
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
