//! Pure authorization predicates, lifted out of the effectful `auth` crate so
//! Aeneas can formally verify them against `spec/BoatschedSpec/Authz.lean`
//! (theorems `AUTH-*`).
//!
//! Everything here is a total function over `bool`/`i64` — no DB, session
//! store, RNG, or `Mutex` — so it extracts cleanly and the proofs cover the
//! actual decision the `auth` crate ships. The effectful shell in
//! `boatsched-auth` (the session store, the `instructor.is_admin` query, the
//! `AdminCtx` capability) resolves the *inputs* to these predicates and then
//! delegates the decision here.

/// The core authorization decision: an actor may perform an admin-gated action
/// iff the server is in **open mode** OR the actor is an **admin**. Mirrors
/// `AuthState.canPerform` in the Lean spec (`openMode || admins m`).
pub fn can_perform(open_mode: bool, member_is_admin: bool) -> bool {
    open_mode || member_is_admin
}

/// Why a set-admin request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetAdminError {
    /// An admin may not revoke their *own* admin status (lockout protection).
    CannotRemoveOwnAdmin,
}

/// Guard for setting an instructor's admin flag: an admin may not revoke their
/// *own* admin status. `caller` is the acting admin; `target`'s flag is being
/// set to `grant`. Mirrors the self-revoke rule the `auth` crate enforced
/// inline (`caller == target && !grant`).
pub fn guard_set_admin(caller: i64, target: i64, grant: bool) -> Result<(), SetAdminError> {
    if caller == target && !grant {
        return Err(SetAdminError::CannotRemoveOwnAdmin);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_perform_truth_table() {
        assert!(can_perform(true, false)); // open mode grants everyone
        assert!(can_perform(true, true));
        assert!(can_perform(false, true)); // admin outside open mode
        assert!(!can_perform(false, false)); // non-admin, closed
    }

    #[test]
    fn cannot_revoke_own_admin() {
        assert_eq!(
            guard_set_admin(7, 7, false),
            Err(SetAdminError::CannotRemoveOwnAdmin)
        );
    }

    #[test]
    fn granting_and_other_targets_ok() {
        assert!(guard_set_admin(7, 7, true).is_ok()); // self-grant is fine
        assert!(guard_set_admin(7, 9, false).is_ok()); // revoking someone else
        assert!(guard_set_admin(7, 9, true).is_ok());
    }
}
