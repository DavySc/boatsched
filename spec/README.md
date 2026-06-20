# boatsched — formal domain spec (Lean 4)

This directory is the **formal model of record** for boatsched's domain. It is a
Lean 4 library that defines the domain types, the rules, and *proves* the
invariants the Rust implementation (`../rust`) must uphold. The goal is to make
agentic changes to the Rust code **safer**: an agent reads the rules here before
touching behaviour, and conformance tests catch any drift.

> **Scope, honestly.** Lean cannot verify arbitrary Rust. The OR-Tools C++ FFI,
> `axum`, `tokio`, `rusqlite`, and `yrs` are out of reach for mechanized proof.
> What *is* formalized is the **pure domain core** — levels, config, validation,
> and the authorization decision — which is where correctness bugs hurt most.

## How it's bound to the Rust code (two tiers)

**Tier 1 — model of record + conformance testing (in place & wired).**
Lean is the source of truth and is proven internally consistent. The Rust crates
implement the same logic and are bound to Lean by **oracle vectors**:
`BoatschedSpec/Conformance.lean` enumerates inputs and the model's verdict,
`lake exe gen-vectors` dumps them to `conformance_vectors.json`, and
`rust/crates/core/tests/conformance.rs` reproduces them against the Rust
`validate`. The link is *testing*, but the thing being tested against is
*proven correct*.

```bash
cd spec && lake exe gen-vectors > conformance_vectors.json   # regenerate oracle
cd ../rust && cargo test -p boatsched-core --test conformance # check Rust agrees
```

Regenerate the JSON whenever the model or its `cases` change, and commit it
alongside the Lean change.

**Tier 2 — mechanized extraction (optional, future).**
[Aeneas](https://github.com/AeneasVerif/aeneas) + Charon can translate a *safe
subset* of Rust into a Lean functional model and let us prove the Rust `validate`
/ level order refine this spec directly — a stronger guarantee than testing, but
only for the pure, FFI-free core. The modules here are organized so that core
(`Level`, `Validate`) is exactly that extractable subset.

## Layout

```
spec/
├── lakefile.toml            # package (depends on Mathlib)
├── lean-toolchain           # pinned: leanprover/lean4:v4.31.0
├── BoatschedSpec.lean       # root: re-exports every module
└── BoatschedSpec/
    ├── Level.lean           # EnrollmentLevel + total order   ↔ core::EnrollmentLevel
    ├── Config.lean          # Config + day ordering           ↔ core::Config
    ├── Validate.lean        # validate + failure taxonomy     ↔ core::validate
    ├── Authz.lean           # capability / admin gate         ↔ crates/auth
    └── Conformance.lean     # oracle vectors for the Rust bridge
```

## Build & check

```bash
cd spec
lake exe cache get  # ONE-TIME: download Mathlib's prebuilt oleans (a few GB)
lake build          # type-checks every definition AND every proof
```

A green build *is* the verification: if any theorem stops holding (e.g. because
the model was changed to match a wrong implementation), `lake build` fails.

> The spec depends on **Mathlib**. The first `lake exe cache get` is heavy, but
> afterwards the dev loop is fast (only our handful of files recompile). CI must
> run `lake exe cache get` before `lake build`.

## Traceability: rule ID ↔ Rust

Every public theorem carries a stable ID. Cite it in the Rust code/tests that
realize the same property, so the spec and implementation stay cross-referenced.

| ID | Theorem (Lean) | Rust counterpart |
|----|----------------|------------------|
| `LVL-1` | `Level.parse_toStr` — parse∘toStr round-trips | `core::EnrollmentLevel::{parse,as_str}` + `enrollment_level_roundtrip` |
| `LVL-2` | `Level.rank_strict_mono` — k<b<A<B | `core` `#[derive(Ord)]` + `enrollment_level_ordering_low_to_high` |
| `CFG-1` | `Config.days_eq` — days = restricted ++ diverse | `core::Config::days` |
| `CFG-2` | `Config.days_empty` — empty iff both empty | `core::Config::days` + `validate` first guard |
| `VAL-1` | `validate_needs_day` — no days ⇒ error | `core::validate` + `validate_no_days_raises` |
| `VAL-2` | `validate_boats_positive` — boats≤0 ⇒ error | `core::validate` + `validate_boats_*_raises` |
| `VAL-3` | `validate_ok_levels_known` — ok ⇒ all levels known (**soundness**) | `core::validate` + `validate_unknown_level_raises` |
| `AUTH-1` | `nonadmin_denied` — auth mode, non-admin ⇒ no action (no priv-esc) | `crates/auth` `AdminCtx` + route gate; `api.rs` 403 tests |
| `AUTH-2` | `open_mode_grants` — open mode ⇒ all actions | `crates/auth` open-mode logic |

## Conventions for changing behaviour

1. **Spec first.** Change the rule in Lean, re-run `lake build`, *then* change the
   Rust to match. Update the conformance vectors if the input/verdict set grows.
2. **One PR.** A behavioural change to `../rust` and its spec change land together.
3. **No `sorry`, no admitted theorems.** A proof that won't go through means the
   property is false or mis-stated — fix the model, don't bypass it.
4. **Prefer trusted libraries over hand-rolled code.** Use Mathlib for proofs
   (`by_contra`, `omega`, `split_ifs`, the order/`List` lemma library) and
   `Lean.Json` / `serde` for serialization. Reach for a hand-rolled solution
   only when a library genuinely doesn't fit — and flag it when you do.

## Open proof obligations (next steps)

- **`AUTH-3` — set-admin monotonicity:** granting admin never removes an existing
  permission. Stub left in `Authz.lean`.
- **Solver invariants:** model the lexicographic objective (A>B>C), the `adj`
  hard constraint on restricted days, and boat capacity, then prove the structural
  invariants the Rust `solver_parity.rs` asserts. (Modelling the *constraints*,
  not the CP-SAT search — the search is nondeterministic and out of scope.)
- **More conformance cases:** the bridge is wired (`lake exe gen-vectors` →
  `conformance_vectors.json` → `core/tests/conformance.rs`); broaden `cases` in
  `Conformance.lean` to cover more of the input space (multi-level rosters,
  boundary divisibility, adj edge cases).
- **Tier 2 spike:** run Charon+Aeneas on `core::validate` and prove it refines
  `BoatschedSpec.validate`.
