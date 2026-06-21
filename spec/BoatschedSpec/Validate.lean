/-
  Roster/config validation.

  Mirrors `validate` in `rust/crates/core/src/lib.rs` (and the Python original):
  a total function returning the *first* failure reason, or `ok`. Error *message
  text* is not modelled here — only the failure *taxonomy* and the order of
  checks, which is what callers depend on.
-/
import BoatschedSpec.Config
import Mathlib.Tactic
namespace Boatsched

/-- A participant. Mirrors `Kid` (`name`, `level`-label from `Config.levels`). -/
structure Kid where
  name : String
  level : String
  deriving Repr

/-- The (ordered) reasons validation can fail. -/
inductive ValidationError where
  | noDays
  | boatsNotPositive
  | notDivisible (kids boats : Int)
  | unknownLevels (bad : List String)
  deriving Repr, DecidableEq

/-- Is `l` one of the configured solver levels? -/
def knownLevel (c : Config) (l : String) : Bool := c.levels.contains l

/-- Validate the roster/config. Total; returns the first failure or `ok ()`.
    The check order matches Rust: days, then boats, then divisibility, then
    unknown levels. -/
def validate (kids : List Kid) (c : Config) : Except ValidationError Unit :=
  if c.days = [] then
    .error .noDays
  else if c.boats ≤ 0 then
    .error .boatsNotPositive
  else if (kids.length : Int) % c.boats ≠ 0 then -- potentially suspect
    .error (.notDivisible (kids.length : Int) c.boats)
  else
    let bad := kids.filterMap (fun k => if knownLevel c k.level then none else some k.level)
    if bad = [] then .ok () else .error (.unknownLevels bad)

/-- [VAL-1] No days configured ⇒ `noDays` (the first guard). -/
theorem validate_needs_day (kids : List Kid) (c : Config) (h : c.days = []) :
    validate kids c = .error .noDays := by
  simp [validate, h]

/-- [VAL-2] Days present but non-positive boats ⇒ `boatsNotPositive`. -/
theorem validate_boats_positive (kids : List Kid) (c : Config)
    (hd : c.days ≠ []) (hb : c.boats ≤ 0) :
    validate kids c = .error .boatsNotPositive := by
  simp [validate, hd, hb]

/-- [VAL-3] Soundness of the happy path: an `ok` result guarantees every kid's
    level is one of the configured levels. -/
theorem validate_ok_levels_known (kids : List Kid) (c : Config)
    (h : validate kids c = .ok ()) :
    ∀ k ∈ kids, knownLevel c k.level = true := by
  intro k hk
  by_contra hbad
  rw [Bool.not_eq_true] at hbad
  -- An unknown-level kid puts its level into the `bad` list.
  have hmem : k.level ∈
      kids.filterMap (fun j => if knownLevel c j.level then none else some j.level) := by
    rw [List.mem_filterMap]
    exact ⟨k, hk, by simp [hbad]⟩
  -- Discharge every if-branch: each error branch contradicts `h = ok`; the
  -- `bad = []` branch contradicts `hmem`.
  simp only [validate] at h
  split_ifs at h
  all_goals simp_all

end Boatsched
