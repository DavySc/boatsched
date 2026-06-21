/-
  `lake exe gen-vectors` — print the conformance oracle (inputs + verdicts) as
  JSON. Regenerate the checked-in vectors with:

      cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json

  The vectors live inside the `rust/` Docker build context (not here) so the
  image build's `cargo test` gate runs the conformance check too. The Rust test
  `rust/crates/core/tests/conformance.rs` embeds that file and asserts the Rust
  `validate` agrees with this proven model.
-/
import BoatschedSpec.Conformance

def main : IO Unit :=
  IO.println Boatsched.Conformance.toJson
