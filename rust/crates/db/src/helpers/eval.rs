use super::members::is_student;
use super::{DbError, Result};
use rusqlite::{params, Connection, OptionalExtension};

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
/// Validation (all → [`DbError::Invalid`]): member is an enrolled non-instructor
/// student; every `day` is a weekday code; each day has ≤2 distinct skipper
/// instructors that are actually instructors; every `skill_id` belongs to the
/// student's enrolled level and each status is legal for that skill's category
/// `eval_type`.
pub fn save_student_progress(conn: &Connection, p: &SaveProgress) -> Result<()> {
    if !is_student(conn, p.member_id, p.season_id)? {
        return Err(DbError::Invalid(
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
            return Err(DbError::Invalid(format!("ongeldige dag: {}", d.day)));
        }
        if d.skippers.len() > 2 {
            return Err(DbError::Invalid("maximaal twee schippers per dag".into()));
        }
        let mut seen = HashSet::new();
        for &iid in &d.skippers {
            if !seen.insert(iid) {
                return Err(DbError::Invalid("dubbele schipper".into()));
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
                return Err(DbError::Invalid(format!(
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
                .map_err(|_| DbError::Invalid(format!("ongeldige skill id: {sid_str}")))?;
            let eval_type = level_skills.get(&sid).ok_or_else(|| {
                DbError::Invalid(format!("skill {sid} hoort niet bij niveau {level}"))
            })?;
            if !allowed_statuses(eval_type).contains(&status.as_str()) {
                return Err(DbError::Invalid(format!(
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
