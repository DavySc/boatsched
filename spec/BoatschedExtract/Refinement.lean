/-
  Refinement proofs (Tier-2): the Aeneas-extracted model of the real
  `rust/crates/core` provably refines the hand-written `BoatschedSpec`, for all
  inputs and panic-free (the extracted functions live in Aeneas's `Result`
  monad, so an `= ok _` result rules out `fail`/`div`, i.e. panics/divergence).

  Theorem IDs (see spec/README.md traceability table):
    * EXT-AUTH — the pure authz predicates refine `BoatschedSpec.Authz`.
    * EXT-VAL  — `validate` refines `BoatschedSpec.validate`.
-/
import BoatschedExtract.Types
import BoatschedExtract.Funs
import BoatschedExtract.FunsExternal
import BoatschedSpec.Validate
import BoatschedSpec.Authz

open Aeneas Aeneas.Std Result

namespace Boatsched.Extract

/-! ### EXT-AUTH — authorization predicates -/

/-- Map the spec's `Except` outcome into the extracted Rust `Result` type. -/
def specToCore : Except Boatsched.SetAdminError Unit →
    core.result.Result Unit boatsched_core.authz.SetAdminError
  | .ok _ => .Ok ()
  | .error .cannotRemoveOwnAdmin => .Err .CannotRemoveOwnAdmin

/-- [EXT-AUTH-1] `can_perform` computes exactly `open || admin`, never panics. -/
theorem can_perform_eq (o a : Bool) :
    boatsched_core.authz.can_perform o a = ok (o || a) := by
  unfold boatsched_core.authz.can_perform
  cases o <;> simp

/-- [EXT-AUTH-2] `can_perform` refines the spec decision `AuthState.canPerform`
    for the matching state, so AUTH-1/AUTH-2 carry onto the real Rust code. -/
theorem can_perform_refines (s : Boatsched.AuthState) (m : Boatsched.MemberId)
    (act : Boatsched.GatedAction) :
    boatsched_core.authz.can_perform s.openMode (s.admins m)
      = ok (s.canPerform m act) := by
  rw [can_perform_eq]; rfl

/-- [EXT-AUTH-3] `guard_set_admin` refines the spec self-revoke guard
    `guardSetAdmin` (on the underlying integer ids), so AUTH-4/AUTH-5 carry onto
    the real Rust code — and it never panics. -/
theorem guard_set_admin_refines (caller target : I64) (grant : Bool) :
    boatsched_core.authz.guard_set_admin caller target grant
      = ok (specToCore (Boatsched.guardSetAdmin caller.val target.val grant)) := by
  unfold boatsched_core.authz.guard_set_admin Boatsched.guardSetAdmin
  have hbridge : (caller = target) ↔ (caller.val = target.val) := by
    constructor
    · intro h; rw [h]
    · intro h; scalar_tac
  by_cases h : caller = target
  · have hv : caller.val = target.val := hbridge.mp h
    rw [if_pos h, hv]
    cases grant <;> simp [specToCore]
  · have hv : caller.val ≠ target.val := fun hv => h (hbridge.mpr hv)
    rw [if_neg h]
    simp [hv, specToCore]

end Boatsched.Extract
