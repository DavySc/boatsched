/-
  `lake exe gen-vectors` — print the conformance oracle (inputs + verdicts) as
  JSON. Regenerate the checked-in vectors with:

      cd spec && lake exe gen-vectors > conformance_vectors.json

  The Rust test `rust/crates/core/tests/conformance.rs` reads that file and
  asserts the Rust `validate` agrees with this proven model.
-/
import BoatschedSpec.Conformance

def main : IO Unit :=
  IO.println Boatsched.Conformance.toJson
