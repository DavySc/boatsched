-- External function models for the Aeneas-extracted `boatsched_core`.
-- Hand-filled from `FunsExternal_Template.lean` (do not delete the template;
-- re-extraction regenerates it, and this file fills the holes it leaves).
--
-- These are the std-library primitives Aeneas leaves opaque. Each is given a
-- *total, faithful* Lean model (not an axiom) so the refinement proofs in
-- `BoatschedExtract.Refinement` can unfold them:
--   * String equality  -> Boolean `==` on `String` (decidable, lawful).
--   * String clone      -> identity (a clone is value-equal to its source).
--   * Vec::is_empty     -> `length = 0`, exactly like Aeneas's own
--                          `core.slice.Slice.is_empty`.
import Aeneas
import BoatschedExtract.Types
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
open boatsched_core

/-- `<String as PartialEq>::eq` — Boolean structural equality on `String`. -/
@[rust_fun
  "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqString.eq
  (a b : String) : Result Bool := ok (a == b)

/-- `<String as Clone>::clone` — a clone equals its source. -/
@[rust_fun "alloc::string::{core::clone::Clone<alloc::string::String>}::clone"]
def alloc.string.String.Insts.CoreCloneClone.clone
  (s : String) : Result String := ok s

/-- `Vec::is_empty` — the vector has length zero. Mirrors Aeneas's own
    `core.slice.Slice.is_empty` (`ok (s.length = 0)`). -/
@[rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::is_empty"]
def alloc.vec.Vec.is_empty
  {T : Type} (A : Type) (v : alloc.vec.Vec T) : Result Bool :=
  ok (v.length = 0)
