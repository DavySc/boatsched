/-
  Conformance bridge: oracle vectors derived from the *proven* model, to be
  reproduced by the Rust implementation (Tier-1 binding — see README.md).

  Each `Vector` pairs an input with the model's expected verdict. A future
  `lake exe gen-vectors` (or a checked-in JSON dump of `#eval expected`) feeds
  `rust/crates/core/tests/conformance.rs`, which asserts the Rust `validate`
  agrees. Until that exists, the `#eval`s below are runnable oracles.
-/
import BoatschedSpec.Validate
import Lean
namespace Boatsched
namespace Conformance
open Lean (ToJson)

/-- The verdict the Rust side must match, as a stable string tag. -/
def verdict : Except ValidationError Unit → String
  | .ok _                   => "ok"
  | .error .noDays          => "noDays"
  | .error .boatsNotPositive => "boatsNotPositive"
  | .error (.notDivisible _ _) => "notDivisible"
  | .error (.unknownLevels _)  => "unknownLevels"

private def cfg (boats : Int) (levels restricted diverse : List String) : Config :=
  { boats, levels, restricted, diverse, adj := 1 }

/-- A named conformance case: id, kids, config. -/
structure Case where
  id : String
  kids : List Kid
  cfg : Config

def cases : List Case :=
  [ { id := "valid_2k_1boat"
    , kids := [⟨"k0", "k"⟩, ⟨"k1", "k"⟩]
    , cfg := cfg 1 ["k"] ["Sat"] [] }
  , { id := "no_days"
    , kids := [⟨"k0", "k"⟩, ⟨"k1", "k"⟩]
    , cfg := cfg 1 ["k"] [] [] }
  , { id := "boats_zero"
    , kids := [⟨"k0", "k"⟩, ⟨"k1", "k"⟩]
    , cfg := cfg 0 ["k"] ["Sat"] [] }
  , { id := "indivisible"
    , kids := [⟨"k0", "k"⟩, ⟨"k1", "k"⟩, ⟨"k2", "k"⟩, ⟨"k3", "k"⟩, ⟨"k4", "k"⟩]
    , cfg := cfg 2 ["k"] ["Sat"] [] }
  , { id := "unknown_level"
    , kids := [⟨"z0", "z"⟩]
    , cfg := cfg 1 ["k"] ["Sat"] [] } ]

/-- The expected (id, verdict) pairs — the oracle for the Rust conformance test.
    Run `lake env lean --run` or just `#eval expected` to print them. -/
def expected : List (String × String) :=
  cases.map (fun c => (c.id, verdict (validate c.kids c.cfg)))

#eval expected
-- [("valid_2k_1boat", "ok"), ("no_days", "noDays"), ("boats_zero",
--  "boatsNotPositive"), ("indivisible", "notDivisible"),
--  ("unknown_level", "unknownLevels")]

/-! ### JSON export

Uses Lean core's `Lean.Json` (`ToJson` derivers) — no hand-rolled serializer.
The derived field names match the Rust `serde` structs in
`rust/crates/core/tests/conformance.rs`. `BoatschedSpec.gen-vectors`
(see `GenVectors.lean`) prints `toJson`. -/

deriving instance ToJson for Kid
deriving instance ToJson for Config

/-- One case as the Rust test consumes it: inputs + the model's verdict. -/
structure CaseOut where
  id : String
  kids : List Kid
  config : Config
  verdict : String
  deriving ToJson

/-- The full oracle as pretty-printed JSON. -/
def toJson : String :=
  Lean.toJson (cases.map fun c =>
    ({ id := c.id, kids := c.kids, config := c.cfg,
       verdict := verdict (validate c.kids c.cfg) } : CaseOut)) |>.pretty

end Conformance
end Boatsched
