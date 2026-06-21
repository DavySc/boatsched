/-
  Problem / solver configuration (domain-relevant fields only).

  Mirrors `Config` in `rust/crates/core/src/lib.rs`. Time budgets and worker
  count are deliberately omitted: they affect *which* optimum the solver returns,
  not the domain invariants, so they are out of scope for the formal model.
-/
import Mathlib.Tactic
namespace Boatsched

structure Config where
  boats : Int
  levels : List String
  restricted : List String
  diverse : List String
  adj : Int
  deriving Repr

/-- Ordered day list: `restricted` then `diverse`. Mirrors `Config::days`.
    The order fixes the day index used throughout the solver. -/
def Config.days (c : Config) : List String := c.restricted ++ c.diverse

/-- [CFG-1] `days` is exactly `restricted ++ diverse` (order preserved). -/
theorem Config.days_eq (c : Config) :
    c.days = c.restricted ++ c.diverse := rfl

/-- [CFG-2] `days` is empty iff both day lists are empty (drives the first
    `validate` check). -/
theorem Config.days_empty (c : Config) :
    c.days = [] ↔ c.restricted = [] ∧ c.diverse = [] := by
  simp [Config.days]

end Boatsched
