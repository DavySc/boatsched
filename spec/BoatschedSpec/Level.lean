/-
  The fixed enrollment-level enum.

  Mirrors `EnrollmentLevel` in `rust/crates/core/src/lib.rs` and the DB
  `CHECK (level IN ('k','b','A','B'))`. Ordered low -> high.
-/
namespace Boatsched

inductive Level where
  | k  -- Ketelbinkie (lowest)
  | b  -- Bemanning
  | A
  | B  -- highest
  deriving DecidableEq, Repr

namespace Level

/-- Rank used for ordering; lower number = lower level. -/
def rank : Level → Nat
  | k => 0
  | b => 1
  | A => 2
  | B => 3

/-- Stored string form used by the DB and JSON API. -/
instance : ToString Level where
  toString
  | k => "k"
  | b => "b"
  | A => "A"
  | B => "B"

/-- Parse a stored level string; case-sensitive, like the DB CHECK. -/
def parse : String → Option Level
  | "k" => some k
  | "b" => some b
  | "A" => some A
  | "B" => some B
  | _   => none

/-- The four levels, low -> high. Mirrors `EnrollmentLevel::ALL`. -/
def all : List Level := [k, b, A, B]

/-- [LVL-1] `parse` is a left inverse of `toString` (round-trip). -/
theorem parse_toStr (l : Level) : parse (toString l) = some l := by
  cases l <;> rfl

/-- [LVL-2] Ranks are strictly increasing: k < b < A < B. -/
theorem rank_strict_mono :
    rank k < rank b ∧ rank b < rank A ∧ rank A < rank B := by
  decide

end Level
end Boatsched
