/-
  Authorization model.

  Mirrors the capability model in `rust/crates/auth` (`AdminCtx`, `CollabRole`)
  and the route-boundary admin gate in `server.py` / `boatsched-server`. The
  Rust side makes illegal states unrepresentable in the *type* system; here we
  prove the corresponding *behavioural* invariants about the decision function.
-/
namespace Boatsched

abbrev MemberId := Int

/-- Sensitive actions gated by the `AdminCtx` capability. Mirrors the admin-only
    operations: run the solver, manage seasons/members/instructors, grant admin,
    and edit boat assignments (`/swap`, `/commit`, collab update frames). -/
inductive GatedAction where
  | runSolver
  | createSeason
  | importMembers
  | renameMember
  | deleteMember
  | createInstructor
  | setAdmin
  | editAssignments
  deriving DecidableEq, Repr

/-- Authorization state. `openMode` is dev-only (no DB / no password set ⇒
    everyone is treated as admin); `admins m` is true iff member `m` is an
    admin instructor. -/
structure AuthState where
  openMode : Bool
  admins : MemberId → Bool

/-- The single authorization decision. Mirrors the route-boundary gate:
    everything is allowed in open mode, otherwise only admins. -/
def AuthState.canPerform (s : AuthState) (m : MemberId) (_a : GatedAction) : Bool :=
  s.openMode || s.admins m

/-- [AUTH-1] In auth mode (open mode off), a non-admin can perform **no** gated
    action. This is the no-privilege-escalation invariant. -/
theorem nonadmin_denied (s : AuthState) (m : MemberId) (a : GatedAction)
    (h_open : s.openMode = false) (h_admin : s.admins m = false) :
    s.canPerform m a = false := by
  simp [AuthState.canPerform, h_open, h_admin]

/-- [AUTH-2] Open mode grants every action (dev parity). -/
theorem open_mode_grants (s : AuthState) (m : MemberId) (a : GatedAction)
    (h_open : s.openMode = true) :
    s.canPerform m a = true := by
  simp [AuthState.canPerform, h_open]

-- TODO [AUTH-3]: `set-admin` is monotone — granting admin never removes an
-- existing permission. Stated in README.md as the next authz proof.

end Boatsched
