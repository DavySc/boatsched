//! Authorization model for boatsched, ported from the auth helpers in
//! `server.py` (`_is_open`, `_auth_member`, `_member_is_admin`,
//! `_current_is_admin`, the session store, and the per-IP login throttle).
//!
//! The whole point of this crate (rewrite invariant: *encode authz in the type
//! system so illegal states are unrepresentable*) is the [`AdminCtx`]
//! capability. It has a private field and is constructed **only** by
//! [`Authenticator::require_admin`] / [`Authenticator::admin_for`] after an
//! actual admin check. Admin-only operations (boat-assignment edits, student-
//! level edits, member/instructor management — stage d routes) take an
//! `&AdminCtx` argument, so a non-admin code path literally cannot call them:
//! it has no way to obtain the token. This replaces the scattered
//! `if not _current_is_admin(): 403` checks in `server.py`.
//!
//! Parity notes that matter:
//! - **Open mode** (`require_auth == false` AND (no DB OR no instructor has a
//!   password)) makes *everyone* admin, and [`Authenticator::auth_member`]
//!   returns `Some(0)` for any/no token — the member-0 sentinel is preserved
//!   exactly (see [`OPEN_MODE_MEMBER`]).
//! - An admin **cannot revoke their own** admin (lockout protection):
//!   [`guard_set_admin`].
//! - Levels are the closed enum `k|b|A|B`, never free text — re-exported as
//!   [`Level`] from `boatsched-core`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};

pub use boatsched_core::EnrollmentLevel as Level;

/// The member id used in **open mode**, mirroring `server.py` where
/// `_auth_member` returns `0` when no passwords are set yet. Real members have
/// positive ids, so `0` is an unambiguous "open/bootstrap" sentinel.
pub const OPEN_MODE_MEMBER: i64 = 0;

/// Tunables that mirror the `create_app` security knobs (defaults match the
/// permissive/dev values so tests behave like the Python suite).
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// When true, open/bootstrap mode is disabled entirely (production).
    pub require_auth: bool,
    /// Seconds before an issued session expires.
    pub session_ttl: Duration,
    /// Per-IP failed-login throttle: max failures within the window.
    pub login_max_failures: usize,
    /// Per-IP failed-login throttle window.
    pub login_window: Duration,
}

impl Default for AuthConfig {
    fn default() -> Self {
        // Mirrors create_app defaults: require_auth=False, session_ttl=12h,
        // login_max_failures=20, login_window=300s.
        Self {
            require_auth: false,
            session_ttl: Duration::from_secs(12 * 3600),
            login_max_failures: 20,
            login_window: Duration::from_secs(300),
        }
    }
}

/// Errors surfaced by the guarded operations in this crate. Variants carry the
/// same status-code intent as the Python responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No valid session and not in open mode → 401.
    NotAuthenticated,
    /// Authenticated but not an admin → 403.
    NotAdmin,
    /// An admin tried to revoke their own admin status → 400.
    CannotRemoveOwnAdmin,
    /// Too many recent failed logins from this IP → 429.
    LoginBlocked,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AuthError::NotAuthenticated => "not authenticated",
            AuthError::NotAdmin => "admin required",
            AuthError::CannotRemoveOwnAdmin => "cannot remove own admin status",
            AuthError::LoginBlocked => "too many login attempts",
        };
        f.write_str(s)
    }
}
impl std::error::Error for AuthError {}

/// Unforgeable proof that the bearer is allowed to perform admin actions.
///
/// Construction is crate-private (the `acting` field is private and there is no
/// public constructor), so the *only* way to get one is through an
/// [`Authenticator`] admin check. Admin-only functions take `&AdminCtx`,
/// making "perform an admin action without being an admin" unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminCtx {
    /// The acting member id. `OPEN_MODE_MEMBER` (0) in open mode, mirroring
    /// `_auth_member` returning 0 there.
    acting: i64,
}

impl AdminCtx {
    /// The member acting under this capability (`OPEN_MODE_MEMBER` in open mode).
    pub fn acting_member(&self) -> i64 {
        self.acting
    }
}

/// A live `/collab/{schedule_id}` websocket participant.
///
/// Mirrors the server rule "non-admins may open the socket and receive updates,
/// but any update frame they send is dropped". Only the [`CollabRole::Editor`]
/// variant carries an [`AdminCtx`], and only it can reach the apply/persist/
/// broadcast branch — so an `Observer` *cannot* mutate the doc by construction.
#[derive(Debug, Clone, Copy)]
pub enum CollabRole {
    /// Non-admin: receives sync/update frames, but sent update frames are dropped.
    Observer,
    /// Admin: may apply, persist, and broadcast updates.
    Editor(AdminCtx),
}

impl CollabRole {
    /// The `AdminCtx` if this participant may edit, else `None` (the apply path
    /// is only reachable when this is `Some`).
    pub fn editor(&self) -> Option<&AdminCtx> {
        match self {
            CollabRole::Editor(ctx) => Some(ctx),
            CollabRole::Observer => None,
        }
    }
}

/// In-memory session + login-throttle store with the auth-mode decision logic.
///
/// Interior mutability: the sessions map and login-failure buckets live behind
/// `Mutex` because in axum the `Authenticator` is shared (`Arc`) across
/// concurrently-running handlers. The locks are held only for the duration of a
/// map lookup/insert, never across `.await`, so a plain `std::sync::Mutex` is
/// the right choice (no async contention, no poisoning concern beyond panics).
pub struct Authenticator {
    config: AuthConfig,
    sessions: Mutex<HashMap<String, Session>>,
    login_failures: Mutex<HashMap<String, Vec<Instant>>>,
}

#[derive(Clone, Copy)]
struct Session {
    member_id: i64,
    expiry: Instant,
}

impl Authenticator {
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
            login_failures: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    // --- mode determination (mirrors _is_open) --------------------------------

    /// True only in dev: `require_auth` is off AND (no DB, or no instructor has
    /// a password yet). In production (`require_auth`) this is always false.
    pub fn is_open(&self, db: Option<&Connection>) -> bool {
        if self.config.require_auth {
            return false;
        }
        match db {
            None => true,
            Some(conn) => {
                let any_password: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM instructor WHERE password_hash IS NOT NULL LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .optional()
                    .unwrap_or(None);
                any_password.is_none()
            }
        }
    }

    // --- session resolution (mirrors _auth_member) ----------------------------

    /// Resolve a session token to a member id. Returns `Some(member_id)` for a
    /// valid, unexpired session; `Some(OPEN_MODE_MEMBER)` in open mode; else
    /// `None`. Expired sessions are evicted on access, exactly like Python.
    pub fn auth_member(&self, token: Option<&str>, db: Option<&Connection>) -> Option<i64> {
        if let Some(tok) = token {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(sess) = sessions.get(tok).copied() {
                if Instant::now() < sess.expiry {
                    return Some(sess.member_id);
                }
                sessions.remove(tok); // expired — evict
            }
        }
        if self.is_open(db) {
            return Some(OPEN_MODE_MEMBER);
        }
        None
    }

    /// True when `mid` is an admin instructor, or when in open mode. Mirrors
    /// `_member_is_admin`.
    pub fn member_is_admin(&self, mid: Option<i64>, db: Option<&Connection>) -> bool {
        if self.is_open(db) {
            return true;
        }
        let (Some(mid), Some(conn)) = (mid, db) else {
            return false;
        };
        if mid == OPEN_MODE_MEMBER {
            // Not open mode (checked above) but a 0 leaked in — never an admin.
            return false;
        }
        let is_admin: Option<bool> = conn
            .query_row(
                "SELECT is_admin FROM instructor WHERE member_id=?",
                [mid],
                |r| r.get::<_, i64>(0).map(|v| v != 0),
            )
            .optional()
            .unwrap_or(None);
        is_admin.unwrap_or(false)
    }

    // --- capability construction ---------------------------------------------

    /// Build an [`AdminCtx`] from a session token, or an error describing why
    /// not. This is the single gate: `Ok` ⇒ the bearer is genuinely an admin
    /// (or open mode). Mirrors `_current_is_admin` but yields a capability
    /// instead of a bool.
    pub fn require_admin(
        &self,
        token: Option<&str>,
        db: Option<&Connection>,
    ) -> Result<AdminCtx, AuthError> {
        let member = self.auth_member(token, db);
        // auth_member is None only when not authenticated *and* not open.
        let mid = member.ok_or(AuthError::NotAuthenticated)?;
        if self.member_is_admin(Some(mid), db) {
            Ok(AdminCtx { acting: mid })
        } else {
            Err(AuthError::NotAdmin)
        }
    }

    /// Like [`require_admin`](Self::require_admin) but from an already-resolved
    /// member id (e.g. when the session was resolved once up-front).
    pub fn admin_for(&self, mid: i64, db: Option<&Connection>) -> Result<AdminCtx, AuthError> {
        if self.member_is_admin(Some(mid), db) {
            Ok(AdminCtx { acting: mid })
        } else {
            Err(AuthError::NotAdmin)
        }
    }

    /// Classify a `/collab` participant: admins become [`CollabRole::Editor`],
    /// everyone else (any authenticated non-admin, or open-mode) becomes
    /// [`CollabRole::Observer`]. Returns `None` only when the participant is not
    /// authenticated at all (cannot even open the socket).
    pub fn collab_role(
        &self,
        token: Option<&str>,
        db: Option<&Connection>,
    ) -> Option<CollabRole> {
        let mid = self.auth_member(token, db)?;
        match self.admin_for(mid, db) {
            Ok(ctx) => Some(CollabRole::Editor(ctx)),
            Err(_) => Some(CollabRole::Observer),
        }
    }

    // --- session lifecycle ----------------------------------------------------

    /// Mint a new session for `member_id` and return its token (mirrors
    /// `_new_session` + `secrets.token_urlsafe(32)`).
    pub fn new_session(&self, member_id: i64) -> String {
        let token = generate_token();
        let expiry = Instant::now() + self.config.session_ttl;
        self.sessions
            .lock()
            .unwrap()
            .insert(token.clone(), Session { member_id, expiry });
        token
    }

    /// Drop a session (logout). No error if the token is unknown.
    pub fn drop_session(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    // --- login throttle (mirrors _record_login_failure / _login_blocked) ------

    /// Record a failed login from `ip`, pruning entries outside the window.
    pub fn record_login_failure(&self, ip: &str) {
        let now = Instant::now();
        let window = self.config.login_window;
        let mut map = self.login_failures.lock().unwrap();
        let bucket = map.entry(ip.to_string()).or_default();
        bucket.retain(|&t| now.duration_since(t) < window);
        bucket.push(now);
    }

    /// Clear an IP's failure counter (a successful login resets it). Mirrors
    /// `login_failures.pop(ip, None)`.
    pub fn clear_login_failures(&self, ip: &str) {
        self.login_failures.lock().unwrap().remove(ip);
    }

    /// True when `ip` has hit the failure cap within the window. Also prunes,
    /// matching the Python implementation's behaviour.
    pub fn login_blocked(&self, ip: &str) -> bool {
        let now = Instant::now();
        let window = self.config.login_window;
        let mut map = self.login_failures.lock().unwrap();
        let bucket = map.entry(ip.to_string()).or_default();
        bucket.retain(|&t| now.duration_since(t) < window);
        bucket.len() >= self.config.login_max_failures
    }
}

/// 32 random bytes, URL-safe base64 without padding — the byte-for-byte shape
/// of `secrets.token_urlsafe(32)` (43 chars).
fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_urlsafe_nopad(&bytes)
}

/// URL-safe base64 (RFC 4648 §5) without padding. Small local impl to avoid a
/// base64 crate dependency for this single use.
fn base64_urlsafe_nopad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

/// Guard for `POST /api/instructors/{id}/set-admin`: an admin may not revoke
/// their *own* admin status (lockout protection). Mirrors the
/// `_auth_member(token) == member_id and not is_admin` check exactly.
///
/// `caller` is the acting admin (from the [`AdminCtx`]); `target` is the
/// instructor whose admin flag is being set to `is_admin`.
pub fn guard_set_admin(caller: &AdminCtx, target: i64, is_admin: bool) -> Result<(), AuthError> {
    if caller.acting_member() == target && !is_admin {
        return Err(AuthError::CannotRemoveOwnAdmin);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boatsched_db::schema::create_tables;
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        boatsched_db::schema::run_migrations(&conn).unwrap();
        conn
    }

    fn add_instructor(conn: &Connection, name: &str, is_admin: bool, with_password: bool) -> i64 {
        conn.execute("INSERT INTO member (name) VALUES (?)", [name])
            .unwrap();
        let mid = conn.last_insert_rowid();
        let pw: Option<Vec<u8>> = if with_password { Some(vec![1, 2, 3]) } else { None };
        conn.execute(
            "INSERT INTO instructor (member_id, password_hash, is_admin) VALUES (?, ?, ?)",
            rusqlite::params![mid, pw, is_admin as i64],
        )
        .unwrap();
        mid
    }

    // ---- open mode -----------------------------------------------------------

    #[test]
    fn open_mode_when_no_db() {
        let a = Authenticator::new(AuthConfig::default());
        assert!(a.is_open(None));
        // everyone is admin; auth_member returns the 0 sentinel
        assert_eq!(a.auth_member(None, None), Some(OPEN_MODE_MEMBER));
        assert!(a.member_is_admin(None, None));
        assert!(a.require_admin(None, None).is_ok());
    }

    #[test]
    fn open_mode_when_db_has_no_passwords() {
        let conn = mem_db();
        add_instructor(&conn, "Ann", true, false); // no password
        let a = Authenticator::new(AuthConfig::default());
        assert!(a.is_open(Some(&conn)));
        assert!(a.require_admin(None, Some(&conn)).is_ok());
    }

    #[test]
    fn require_auth_disables_open_mode() {
        let a = Authenticator::new(AuthConfig {
            require_auth: true,
            ..AuthConfig::default()
        });
        assert!(!a.is_open(None));
        assert_eq!(a.auth_member(None, None), None);
        assert_eq!(a.require_admin(None, None), Err(AuthError::NotAuthenticated));
    }

    // ---- auth mode -----------------------------------------------------------

    #[test]
    fn auth_mode_once_a_password_is_set() {
        let conn = mem_db();
        let admin = add_instructor(&conn, "Ann", true, true);
        let plain = add_instructor(&conn, "Bob", false, true);
        let a = Authenticator::new(AuthConfig::default());
        assert!(!a.is_open(Some(&conn)));

        // No session → not authenticated.
        assert_eq!(a.auth_member(None, Some(&conn)), None);
        assert_eq!(
            a.require_admin(None, Some(&conn)),
            Err(AuthError::NotAuthenticated)
        );

        // Admin session → AdminCtx.
        let tok_admin = a.new_session(admin);
        let ctx = a.require_admin(Some(&tok_admin), Some(&conn)).unwrap();
        assert_eq!(ctx.acting_member(), admin);

        // Non-admin session → NotAdmin.
        let tok_plain = a.new_session(plain);
        assert_eq!(
            a.require_admin(Some(&tok_plain), Some(&conn)),
            Err(AuthError::NotAdmin)
        );
        // …but they are authenticated.
        assert_eq!(a.auth_member(Some(&tok_plain), Some(&conn)), Some(plain));
    }

    #[test]
    fn expired_session_is_evicted() {
        let conn = mem_db();
        let admin = add_instructor(&conn, "Ann", true, true);
        let a = Authenticator::new(AuthConfig {
            session_ttl: Duration::from_millis(0),
            ..AuthConfig::default()
        });
        let tok = a.new_session(admin);
        // ttl 0 → already expired.
        assert_eq!(a.auth_member(Some(&tok), Some(&conn)), None);
    }

    #[test]
    fn logout_drops_session() {
        let conn = mem_db();
        let admin = add_instructor(&conn, "Ann", true, true);
        let a = Authenticator::new(AuthConfig::default());
        let tok = a.new_session(admin);
        assert_eq!(a.auth_member(Some(&tok), Some(&conn)), Some(admin));
        a.drop_session(&tok);
        assert_eq!(a.auth_member(Some(&tok), Some(&conn)), None);
    }

    // ---- collab role ---------------------------------------------------------

    #[test]
    fn collab_admin_is_editor_nonadmin_is_observer() {
        let conn = mem_db();
        let admin = add_instructor(&conn, "Ann", true, true);
        let plain = add_instructor(&conn, "Bob", false, true);
        let a = Authenticator::new(AuthConfig::default());
        let tok_admin = a.new_session(admin);
        let tok_plain = a.new_session(plain);

        assert!(matches!(
            a.collab_role(Some(&tok_admin), Some(&conn)),
            Some(CollabRole::Editor(_))
        ));
        let observer = a.collab_role(Some(&tok_plain), Some(&conn)).unwrap();
        assert!(matches!(observer, CollabRole::Observer));
        assert!(observer.editor().is_none()); // apply path unreachable

        // Unauthenticated cannot open the socket at all.
        assert!(a.collab_role(None, Some(&conn)).is_none());
    }

    // ---- set-admin self-revoke guard ----------------------------------------

    #[test]
    fn cannot_remove_own_admin() {
        let conn = mem_db();
        let admin = add_instructor(&conn, "Ann", true, true);
        let other = add_instructor(&conn, "Bob", true, true);
        let a = Authenticator::new(AuthConfig::default());
        let tok = a.new_session(admin);
        let ctx = a.require_admin(Some(&tok), Some(&conn)).unwrap();

        // Revoking own admin → error.
        assert_eq!(
            guard_set_admin(&ctx, admin, false),
            Err(AuthError::CannotRemoveOwnAdmin)
        );
        // Granting self admin is fine; revoking *another* admin is fine.
        assert!(guard_set_admin(&ctx, admin, true).is_ok());
        assert!(guard_set_admin(&ctx, other, false).is_ok());
    }

    // ---- login throttle ------------------------------------------------------

    #[test]
    fn login_throttle_blocks_after_max_failures() {
        let a = Authenticator::new(AuthConfig {
            login_max_failures: 3,
            ..AuthConfig::default()
        });
        let ip = "1.2.3.4";
        assert!(!a.login_blocked(ip));
        a.record_login_failure(ip);
        a.record_login_failure(ip);
        assert!(!a.login_blocked(ip)); // 2 < 3
        a.record_login_failure(ip);
        assert!(a.login_blocked(ip)); // 3 >= 3
        // A different IP is unaffected.
        assert!(!a.login_blocked("5.6.7.8"));
    }

    #[test]
    fn login_throttle_prunes_outside_window() {
        let a = Authenticator::new(AuthConfig {
            login_max_failures: 2,
            login_window: Duration::from_millis(0), // everything is "outside"
            ..AuthConfig::default()
        });
        let ip = "1.2.3.4";
        a.record_login_failure(ip);
        a.record_login_failure(ip);
        a.record_login_failure(ip);
        // window 0 → all prior entries pruned on check → never blocks.
        assert!(!a.login_blocked(ip));
    }

    // ---- token shape ---------------------------------------------------------

    #[test]
    fn session_tokens_are_urlsafe_and_unique() {
        let a = Authenticator::new(AuthConfig::default());
        let t1 = a.new_session(1);
        let t2 = a.new_session(1);
        assert_ne!(t1, t2);
        // token_urlsafe(32) → 43 chars, URL-safe alphabet, no padding.
        assert_eq!(t1.len(), 43);
        assert!(t1
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
