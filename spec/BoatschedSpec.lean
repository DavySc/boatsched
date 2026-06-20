/-
  boatsched — formal domain model (Lean 4).

  Root module: re-exports every part of the spec. The spec is the *model of
  record*; the Rust crates under ../rust implement it and are bound to it by the
  conformance vectors in `BoatschedSpec.Conformance` (see README.md).

  Each public theorem carries a stable ID (e.g. `LVL-1`, `AUTH-1`) referenced
  from the traceability table in README.md and, ideally, from the Rust test that
  exercises the same property.
-/
import BoatschedSpec.Level
import BoatschedSpec.Config
import BoatschedSpec.Validate
import BoatschedSpec.Authz
import BoatschedSpec.Conformance
