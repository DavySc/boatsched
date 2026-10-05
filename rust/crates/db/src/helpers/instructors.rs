use super::{DbError, Result};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstructorRow {
    pub id: i64,
    pub name: String,
    /// Kept as integer to match the JSON the Python endpoint emits (tests
    /// compare `== 1` / `== 0`).
    pub is_admin: i64,
    pub has_password: i64,
}

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
        return Err(DbError::Invalid(
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
