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
`lake exe gen-vectors` dumps them to the checked-in
`rust/crates/core/tests/conformance_vectors.json`, and
`rust/crates/core/tests/conformance.rs` embeds + reproduces them against the Rust
`validate`. The link is *testing*, but the thing being tested against is
*proven correct*. The vectors deliberately live under `rust/` (not here) so they
sit inside the Docker build context — the image build's `cargo test` gate runs
the conformance check too.

```bash
cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json
cd ../rust && cargo test -p boatsched-core --test conformance # check Rust agrees
```

Regenerate the JSON whenever the model or its `cases` change, and commit it
alongside the Lean change.

**Tier 2 — mechanized extraction (in place for `validate` + authz).**
[Charon](https://github.com/AeneasVerif/charon) +
[Aeneas](https://github.com/AeneasVerif/aeneas) translate the **actual**
`rust/crates/core` functions into a Lean functional model (in Aeneas's `Result`
monad), and `BoatschedExtract/Refinement.lean` **proves that model refines this
spec** — for *all* inputs, not sampled ones, and panic-free (an `= ok _` result
in the `Result` monad rules out `fail`/`div`). This is strictly stronger than the
Tier-1 vectors. Proven today:

- **`EXT-AUTH`** — `authz::can_perform` = `open ‖ admin` and refines
  `AuthState.canPerform`; `authz::guard_set_admin` refines the self-revoke guard.
- **`EXT-VAL`** — `validate` refines `BoatschedSpec.validate` (all four guards),
  under two physically-always-true bounds carried as hypotheses: the roster fits
  an `i64` and the day list fits a `usize`.

Only the pure, FFI-free core is reachable this way; the model is checked in under
`BoatschedExtract/` and rebuilt by `lake build` like everything else. `#print
axioms` on the refinement theorems shows only `propext/Classical.choice/Quot.sound`
— no `sorry`.

### Regenerating the extracted model

The model in `BoatschedExtract/{Types,Funs}.lean` is generated; the externals in
`FunsExternal.lean` and the proofs in `Refinement.lean` are hand-written. To
re-extract after changing the verified Rust functions (pinned toolchain:
Charon/Aeneas `nightly-2026.07.21-a827e6f`, rust `nightly-2026-06-01`; both on
lean4 `v4.31.0` + Mathlib `v4.31.0`, identical to this spec):

```bash
# in a crate holding the real core source, with charon/charon-driver on PATH:
charon cargo --preset=aeneas \
  --start-from 'boatsched_core::validate' \
  --start-from 'boatsched_core::authz::can_perform' \
  --start-from 'boatsched_core::authz::guard_set_admin'
aeneas -backend lean -split-files -loops-to-rec -subdir BoatschedExtract \
  -dest <spec-dir> boatsched_core.llbc
```

`--start-from` keeps extraction to the clean cone (`Config`/`f64`/serde/`Display`
never enter); `-loops-to-rec` emits loops as recursive functions (the shape the
proofs reason about). Re-extraction currently a documented manual step — a CI
*freshness gate* (re-run + `git diff --exit-code`) is the next hardening step.

## Layout

```
spec/
├── lakefile.toml            # package (depends on Mathlib)
├── lean-toolchain           # pinned: leanprover/lean4:v4.31.0
├── BoatschedSpec.lean       # root: re-exports every module
├── BoatschedSpec/
│   ├── Level.lean           # EnrollmentLevel + total order   ↔ core::EnrollmentLevel
│   ├── Config.lean          # Config + day ordering           ↔ core::Config
│   ├── Validate.lean        # validate + failure taxonomy     ↔ core::validate
│   ├── Authz.lean           # capability / admin gate         ↔ crates/auth
│   └── Conformance.lean     # oracle vectors for the Rust bridge
├── BoatschedExtract.lean    # root of the Tier-2 extraction library
└── BoatschedExtract/        # Charon+Aeneas model of the REAL core + refinement
    ├── Types.lean           # generated: extracted types (do not edit)
    ├── Funs.lean            # generated: extracted functions (do not edit)
    ├── FunsExternal.lean    # hand-filled models of std primitives (String eq/clone, Vec::is_empty)
    └── Refinement.lean      # EXT-AUTH / EXT-VAL: the extracted core refines BoatschedSpec
```

## Build & check

```bash
cd spec
lake exe cache get  # ONE-TIME: download Mathlib's prebuilt oleans (a few GB)
lake build          # type-checks every definition AND every proof
```

A green build *is* the verification: if any theorem stops holding (e.g. because
the model was changed to match a wrong implementation), `lake build` fails.

> The spec depends on **Mathlib** and (for `BoatschedExtract`) on **Aeneas**'s
> Lean library. The first `lake exe cache get` is heavy, but afterwards the dev
> loop is fast (only our handful of files recompile). Aeneas is pinned to a
> release tag with `preferReleaseBuild`, so `lake build` **downloads** its
> prebuilt oleans rather than compiling them.
>
> **This runs in CI:** the `verify-spec` job in `.forgejo/workflows/deploy.yml`
> installs Lean via elan, runs `lake exe cache get`, then `lake build` on every
> PR and every push — the first time the spec is gated in CI. It is purely
> additive (it does not gate `deploy`); make it a **required check** in branch
> protection so a proof regression blocks merge.

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
| `AUTH-4` | `guardSetAdmin_denied_iff` — refused iff self-revoke | `core::authz::guard_set_admin` + `cannot_revoke_own_admin` |
| `AUTH-5` | `guardSetAdmin_grant_ok` — granting always allowed | `core::authz::guard_set_admin` + `granting_and_other_targets_ok` |

### Tier-2 refinement (extracted Rust ⊑ spec)

These bind the **Charon+Aeneas model of the real Rust** (`BoatschedExtract`) to
the spec above — proven for all inputs, panic-free. See `Refinement.lean`.

| ID | Theorem (Lean) | Rust counterpart |
|----|----------------|------------------|
| `EXT-AUTH` | `can_perform_eq` / `can_perform_refines` / `guard_set_admin_refines` — extracted authz refines `AuthState.canPerform` + `guardSetAdmin` | `core::authz::{can_perform, guard_set_admin}` |
| `EXT-VAL` | `validate_refines` — extracted `validate` refines `BoatschedSpec.validate` (∀ inputs, panic-free) | `core::validate` (+ `ProblemConfig::{days, all_known, knows_level}`) |

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
  permission. Stub left in `Authz.lean`. (`AUTH-4/5` on the self-revoke guard are
  now proven.)
- **`EXT-LVL` — level order:** extend the Tier-2 extraction to `EnrollmentLevel`
  and prove its (derived) order refines `Level.rank`. It is outside `validate`'s
  extraction cone, so it needs its own `--start-from`.
- **Re-extraction freshness gate:** a CI step that re-runs Charon+Aeneas and
  `git diff --exit-code`s the regenerated `BoatschedExtract/{Types,Funs}.lean`
  against the checked-in copy, so a change to a verified Rust function that isn't
  re-extracted fails. Until it lands, regeneration is the documented manual step
  above — the one remaining gap in the "fully mechanical" guarantee.
- **Solver invariants:** model the lexicographic objective (A>B>C), the `adj`
  hard constraint on restricted days, and boat capacity, then prove the structural
  invariants the Rust `solver_parity.rs` asserts. (Modelling the *constraints*,
  not the CP-SAT search — the search is nondeterministic and out of scope.)
- **More conformance cases:** the bridge is wired (`lake exe gen-vectors` →
  `conformance_vectors.json` → `core/tests/conformance.rs`); broaden `cases` in
  `Conformance.lean` to cover more of the input space (multi-level rosters,
  boundary divisibility, adj edge cases).
