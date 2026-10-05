use super::{DbError, Result};
use boatsched_core::ScheduleRow;
use rusqlite::{params, Connection, OptionalExtension};

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
/// unknown name ⇒ [`DbError::Missing`]. Atomic (rolls back on error). Mirrors
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
            .ok_or_else(|| DbError::Missing(format!("member '{}' not found in DB", row.name)))?;
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

/// Schedule rows; [`DbError::Missing`] if any member lacks an enrollment in the
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
        return Err(DbError::Missing(format!(
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

/// Swap two members' boats on a given day. [`DbError::Missing`] if either is
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
            return Err(DbError::Missing(
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
/// unknown name ⇒ rollback + [`DbError::Missing`]. `assignments` are
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
            member_id.ok_or_else(|| DbError::Missing(format!("member '{name}' not found")))?;
        tx.execute(
            "INSERT INTO boat_assignment (schedule_id, member_id, boat_id, day) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_id, member_id, boat_id, day],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// The season a schedule belongs to; `None` for an unknown id.
pub fn get_schedule_season(conn: &Connection, schedule_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT season_id FROM schedule WHERE id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The raw `schedule.config` JSON; `None` for an unknown id or a NULL column.
pub fn get_schedule_config(conn: &Connection, schedule_id: i64) -> Result<Option<String>> {
    let v: Option<Option<String>> = conn
        .query_row(
            "SELECT config FROM schedule WHERE id=?1",
            params![schedule_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.flatten())
}
