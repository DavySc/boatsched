# boatsched — architecture map and contributor workflow

This document has two jobs: give you a real mental model of how the system fits
together, and — more importantly — tell you *which files to touch, in what
order, and which gates you'll trip* for a given kind of change. It assumes no
prior context with the repo.

It deliberately doesn't restate what other docs already own:
[`README.md`](../README.md) is the user-facing overview and local-dev
quickstart, [`deploy/README.md`](../deploy/README.md) is the authoritative
deploy runbook, [`spec/README.md`](../spec/README.md) owns the Lean spec's own
conventions, and [`CLAUDE.md`](../CLAUDE.md) is the terse reference a coding
agent loads every session. This document is the map that tells you which of
those to open next, plus the connective tissue (the *why* and the *order of
operations*) that none of them individually cover.

## 1. System map

boatsched is a single Rust binary (Axum) that serves an embedded single-page
app, stores everything in one SQLite file, delegates constraint solving to
Google OR-Tools CP-SAT through a small C++ FFI shim, and supports real-time
collaborative schedule editing over a Yjs CRDT. A Lean 4 project (`spec/`) is
the formal model of record for the pure domain core, mechanically checked
against the real Rust in CI. Production is one Docker container behind nginx.

```
                        ┌───────────────────────────┐
                        │        spec/ (Lean 4)      │  formal model of record
                        │  Tier 1: conformance       │  for core/authz
                        │  Tier 2: Charon+Aeneas     │
                        │  extraction + refinement   │
                        └─────────────┬─────────────┘
                                      │ proves / checked against
                                      ▼
┌──────────────────────────────────────────────────────────────────┐
│  rust/crates/core        pure domain types, zero deps (the leaf) │
│    EnrollmentLevel · Config/ProblemConfig · validate · authz     │
└───────┬───────────────┬───────────────┬──────────────────────────┘
        │               │               │
        ▼               ▼               ▼
  crates/roster     crates/db      crates/auth ──depends on──▶ crates/db
  (CSV ingest)     (SQLite,        (sessions, scrypt,
                    migrations)     AdminCtx capability)
        │               │               │
        └───────┬───────┴───────┬───────┘
                ▼               ▼
          crates/solver   (all of the above)
        (OR-Tools CP-SAT        │
         via C++ FFI)           ▼
                │         crates/server (binary: boatsched-server)
                └────────▶ Axum HTTP + WebSocket app
                           serves the SPA built from web/
```

### Crate responsibilities and the dependency graph

| Crate | Depends on | Responsibility |
|---|---|---|
| `core` | — (leaf) | Pure domain types and the **verified surface**: `EnrollmentLevel`, `Config`/`ProblemConfig`, `validate`, `authz::{can_perform, guard_set_admin}`. |
| `roster` | `core` | CSV roster ingestion (file + raw upload bytes + synthetic-from-counts). |
| `db` | `core` | All SQLite access: schema, idempotent migrations, query helpers. |
| `auth` | `core`, `db` | Session store, scrypt password hashing, login throttle, the `AdminCtx` capability. |
| `solver` | `core` | The CP-SAT model via a C++ FFI shim; orchestrates the lexicographic A→B→C stages. |
| `server` | all of the above | The Axum app factory + `boatsched-server` binary; owns every route and the embedded SPA. |

`core` is the only crate with zero path-dependencies — everything else builds
on it, and it's the crate the Lean spec formally binds to. `server` is the only
crate that produces a binary; nothing depends on it.

Key types worth knowing by name before you touch anything:

- `core::Config` (`rust/crates/core/src/lib.rs:50`) — the full solver config,
  including `f64` timing fields (`stage_seconds`, `seconds`). It stays flat and
  byte-identical to its stored shape because it's serialized
  verbatim into the `schedule.config` TEXT column (byte-compatibility
  constraint — see §2c).
- `core::ProblemConfig` (`lib.rs:108`) — the **float-free projection** of
  `Config` (`boats, levels, restricted, diverse, adj`). This is the *only*
  config type that crosses into the Lean/Aeneas verification boundary, because
  Aeneas's `F64` type can't appear inside an extracted struct. `Config::problem()`
  (`lib.rs:83`) is the (cheap, non-serialized) projection function. If you're
  ever tempted to add a new tunable to `Config`, ask whether `validate` needs it
  — if yes, it goes on `ProblemConfig`, not as an extra float bolted onto `Config`.
- `core::validate` (`lib.rs:231`) and `core::ValidationError` (`lib.rs:187`) —
  the roster/config feasibility check. The four-variant error enum
  (`NoDays | BoatsNotPositive | NotDivisible{..} | UnknownLevels`) mirrors the
  Lean taxonomy in `spec/BoatschedSpec/Validate.lean` **exactly**; the check
  order (days → boats → divisibility → unknown levels) is load-bearing. Written
  in an unusual index-loop style (no iterators/`HashSet`/`format!`) — that's not
  incidental style, it's a hard requirement for Aeneas extraction (see §2a).
- `core::authz::{can_perform, guard_set_admin}` (`rust/crates/core/src/authz.rs`)
  — the *pure* authorization decisions, lifted out of the effectful `auth`
  crate specifically so they can be formally verified. `auth::Authenticator`
  resolves real inputs (open-mode flag, DB `is_admin` column) and delegates the
  actual decision here.
- `auth::AdminCtx` (`rust/crates/auth/src/lib.rs:99`) — a capability type with a
  **private field and no public constructor**. The only way to get one is
  through `Authenticator::require_admin`/`admin_for`. Every admin-only server
  handler takes `&AdminCtx`, which makes "call an admin action without being an
  admin" a compile error, not a runtime check you might forget. `CollabRole`
  (`lib.rs:119`) extends the same pattern to the `/collab` WebSocket: only the
  `Editor(AdminCtx)` variant can reach the doc-mutation branch.
- `server::AppState` / `create_app` (`rust/crates/server/src/lib.rs`) —
  the shared state (`Config`, an `Option<Arc<Mutex<Connection>>>` for the single
  DB connection, the `Authenticator`, the in-memory solve-job map, Yjs
  `CollabState`) and the function that wires up the ~28-route `Router` plus the
  one global `auth_middleware` layer.

### Persistence

SQLite, one file, opened with `journal_mode=WAL` + `foreign_keys=ON`
(`rust/crates/db/src/lib.rs:17`). The schema is frozen: `SCHEMA_SQL`
(`rust/crates/db/src/schema.rs:11`) is reproduced **verbatim** from the original
schema and is explicitly marked "do not modify" — see §2c for
what that means in practice. Tables: `member`, `instructor`, `season`,
`schedule`, `boat`, `boat_assignment`, `enrollment` (level `CHECK IN
('k','b','A','B')`, one row per member per season), plus a legacy
`skill`/`skill_result`/`v_level_card` trio that predates the current evaluation
feature and a newer, actually-used `eval_skill_category`/`eval_skill`/
`eval_assessment`/`eval_skipper`/`eval_note` set seeded from a static Rust
catalog (`db::eval_catalog`, `rust/crates/db/src/eval_catalog.rs`). All schema
evolution happens through additive, idempotent migration functions chained in
`run_migrations()` — never by editing `SCHEMA_SQL` in place.

> **Documentation currency note.** `CLAUDE.md` currently lists a "known gap":
> that the SPA's evaluation cards call `GET`/`POST /api/progress` but no
> such route exists. That's stale — both routes are registered
> (`rust/crates/server/src/lib.rs`, handlers `get_progress`/`post_progress`
> in `routes/eval.rs`) and backed by `db::helpers::eval::{list_skill_catalog,
> get_season_progress, save_student_progress}` and the `eval_*` tables above.
> Treat any doc (including this one, in a year) as a claim to verify against
> the code, not a fact — `grep` before you trust.

### Solver internals

`rust/crates/solver` is a thin Rust orchestration layer over a C++ CP-SAT model.
`build.rs` locates the pinned OR-Tools SDK, compiles `cpp/cpsat_shim.cpp`, and
bakes an rpath so `libortools.so` is found at runtime without `LD_LIBRARY_PATH`
— `server/build.rs` has to redo the same rpath dance because it links the FFI
transitively. The shim exposes one `extern "C"` function,
`bs_cpsat_solve(...)`, over flat primitive arrays; its signature must stay in
lockstep with the `extern "C" { fn bs_cpsat_solve }` declaration in
`solver/src/lib.rs:14`.

The model builds a boolean cube `x[kid][day][boat]`, then solves **three
lexicographic stages**, locking each stage's optimum as a hard constraint
before the next stage runs (`cpp/cpsat_shim.cpp:130` onward):

| Stage | Objective | Applies to |
|---|---|---|
| A | Minimize mixed-level boats | `restricted` days (default Sat/Sun/Tue/Wed) |
| B | Maximize distinct levels per boat | `diverse` days (default Fri/Mon) |
| C | Minimize repeat co-boatings (social-golfer) | all days |

The `adj` config value is a *hard* constraint throughout: on restricted days, a
boat may only mix levels whose indices differ by at most `adj`. Two
performance accelerators exist (a level "prefixing" symmetry-breaker for Stage
A, and pair-pruning for Stage C) and Stage C gets a warm-start hint seeded from
Stage B's solution. `solver::solve` (`solver/src/lib.rs:86`) calls
`core::validate` first, so a structurally-infeasible roster/config never
reaches the FFI call at all.

### Request and collaboration flow

Every request passes through one global `auth_middleware`
(`server/src/http.rs`) that resolves the session cookie, denies
non-public paths without one, and stamps security headers on the way out.
`/collab/*` is exempted from that middleware (the WS-upgrade handler does its
own auth) because axum's HTTP middleware doesn't run for the WebSocket scope.
DB-touching handlers take the `Db` extractor (503 when there is no DB) and call
`db.run(|conn| ...)`, which runs the closure on `spawn_blocking` over the single
shared `Arc<Mutex<Connection>>` — one connection, not a connection pool.
Handlers return `Result<Response, AppError>` (`server/src/error.rs`), so `?`
maps failures to responses.

`POST /solve` kicks off a UUID-keyed background job (`tokio::spawn` +
`spawn_blocking` for the actual CP-SAT call); `GET /status?id=` polls it. On
success the server persists the schedule and initializes a Yjs document from
the solved rows. `/collab/{schedule_id}` then speaks the y-websocket binary
sync protocol (`server/src/collab.rs`): one `yrs::Doc` per open schedule, a
`Y.Map` keyed `"{day}:{boat}"` holding `Y.Array<name>`, fanned out to
subscribers via a `broadcast::Sender`. Non-admin participants can open the
socket and receive updates, but any update frame they send is dropped — the
`CollabRole`/`AdminCtx` capability gate applies here too, at the point of use.
"Commit schedule" (`POST /schedule/{id}/commit`) reads the live (or persisted)
Yjs state, checks for same-day double-bookings, and writes it into
`boat_assignment` as the durable source of truth.

### Frontend

`web/` is a React + Vite + TypeScript app. `npm run build` type-checks and bundles
it into a single self-contained `web/dist/index.html` (via `vite-plugin-singlefile`;
Yjs and y-websocket are npm dependencies, no CDN), which
`rust/crates/server/build.rs` copies into the binary (`include_str!` of the copy in
`OUT_DIR`; a placeholder page is embedded when `web/dist` doesn't exist, so
`cargo test` needs no Node). That is *why* the Docker build context is the repo
root and why the Dockerfile has a Node stage.

Layout: `session.tsx` (login state, config, boat names), `Shell.tsx` (season and
schedule pickers, solve/re-solve jobs, modals), `schedule/` (the planning screen),
`eval/` (evaluation card), `admin/` (members, instructors), `api.ts` (one fetch
wrapper: `r.ok` handling, global 401 → login), `i18n.ts` (Dutch day/level names).

The live schedule is a Yjs doc (`schedule/useYSchedule.ts`) synced through
`/collab/{id}`: map `schedule` (`"{day}:{boat}"` → names, what `/commit` writes),
plus maps `locks` and `absences` (`name|day` → true, inputs for
`POST /schedule/{id}/resolve`). All edits go through that doc (moves, swaps,
locks, absences, undo/redo via `Y.UndoManager`); only admins' updates are kept by
the server. `schedule/conflicts.ts` recomputes violations (double or missing
placement, absent but placed, level spread on restricted days, boat sizes)
from the doc on every change — the same rules the solver enforces.

### Verification layer (`spec/`)

A Lean 4 project, bound to the real Rust in two tiers:

- **Tier 1 — conformance vectors** (all of `core`): `spec/BoatschedSpec/*.lean`
  is a proven, from-scratch reimplementation of the domain rules (levels,
  `Config.days`, `validate`'s taxonomy, the authorization gate). Its
  `Conformance.lean` module enumerates test cases, `lake exe gen-vectors` dumps
  them to `rust/crates/core/tests/conformance_vectors.json`, and
  `rust/crates/core/tests/conformance.rs` replays them against the *actual*
  Rust `validate`. This is testing against a proven oracle, not a proof of the
  Rust itself.
- **Tier 2 — Charon+Aeneas refinement** (`validate` + `authz` only): the *real*
  `rust/crates/core` source is mechanically extracted (Charon → LLBC → Aeneas →
  Lean) into `spec/BoatschedExtract/`, and `Refinement.lean` **proves** that
  extracted model refines the Tier-1 spec, for all inputs, panic-free
  (`EXT-VAL`, `EXT-AUTH`). This is strictly stronger than Tier 1 — it's not
  sampled, and it separately rules out panics/overflow. Because it only reaches
  a small, deliberately-shaped slice of `core`, `validate`/`ProblemConfig`/
  `authz` are written in that Aeneas-friendly style (index loops, no
  iterators/`HashSet`/`format!`, no `f64` in the extracted struct) — **that
  style is load-bearing, not a stylistic quirk**, and it must be kept if you
  touch those functions (§2a).

`cd spec && lake build` is the check for both tiers — a green build *is* the
proof. It runs in CI as the `verify-spec` job.

### Ops layer

Production is one Docker container (`rust/Dockerfile`, both stages
`ubuntu:24.04` to match the pinned OR-Tools SDK's ABI), behind nginx+certbot,
managed by a systemd unit that runs `docker compose` in the foreground so
`Restart=on-failure` works. CI (GitHub Actions) runs three jobs: `test`
(blocking, `cargo test --workspace`), `verify-spec` (additive, `lake build`),
and `deploy` (`needs: test`, main-only, SSHes to the box and runs
`deploy/deploy.sh`). Full detail in §2g and `deploy/README.md`.

---

## 2. The natural flow of development

This is organized by *kind of change*. For each, the order matters — skipping a
step is usually exactly the mistake that trips a CI gate later.

### a. Changing a domain rule (levels, `validate`, `authz`)

This is the one place "spec first" is a hard rule in this repo, not a
suggestion (`CLAUDE.md`, `spec/README.md` convention #1):

1. Edit the rule in `spec/BoatschedSpec/{Level,Config,Validate,Authz}.lean`
   first. Run `cd spec && lake build` — this both type-checks and re-proves
   the relevant theorem(s) (`LVL-*`, `CFG-*`, `VAL-*`, `AUTH-*`). If a proof
   won't go through, the rule is probably mis-stated — fix the model, don't
   route around it (no `sorry`, ever, in this repo).
2. Mirror the change in `rust/crates/core` (`lib.rs` for levels/`validate`/
   `Config`/`ProblemConfig`, `authz.rs` for the authorization predicates). If
   you're touching `validate`/`ProblemConfig`/`authz`, keep the extraction-
   friendly style: explicit index loops, no iterators/`HashSet`/`format!`
   inside the verified functions (put human-readable text in a `Display` impl
   outside the boundary, as `ValidationError` already does), and keep
   `ProblemConfig` float-free.
3. If the case/input space grew, regenerate the conformance vectors:
   `cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json`.
4. `cd rust && cargo test --workspace` — this runs the conformance test
   (`core/tests/conformance.rs`) along with everything else.
5. **If you changed the actual logic or signature of `validate`, `ProblemConfig`,
   or `authz::{can_perform, guard_set_admin}`** (not just called them the same
   way), the Tier-2 extracted model in `spec/BoatschedExtract/{Types,Funs}.lean`
   is now stale relative to the source. Re-run the extraction (exact pinned
   command in `spec/README.md`'s "Regenerating the extracted model" section —
   `charon cargo --preset=aeneas --start-from '<fn>' ...` then
   `aeneas -backend lean -split-files -loops-to-rec ...`), then `lake build`
   again to confirm `Refinement.lean` still proves. **This step is currently a
   manual, documented gap** — there is no CI freshness check yet that would
   catch "logic changed but the extracted model wasn't regenerated" (see
   `spec/README.md`'s open obligations). Don't skip it just because CI won't
   catch you skipping it.

### b. Adding or changing an HTTP route

1. Register the route in `create_app`'s `Router::new()` chain
   (`rust/crates/server/src/lib.rs`) and write the handler in the matching
   `rust/crates/server/src/routes/*.rs` module.
2. Decide the auth shape: does it need a session at all (add to
   `PUBLIC_PATHS` if not), and is it admin-only? Admin-only handlers should
   call `db.admin(&headers, denied_error)` and thread the resulting
   `AdminCtx` through to any mutation — don't hand-roll an `is_admin` check,
   the whole point of the capability type is that a privileged DB write is
   unreachable without one.
3. DB-touching handlers take the `Db` extractor and use `db.run(...)`; return
   `Handled` and map errors with `AppError` — follow the nearest existing handler.
4. Add/extend a test in `rust/crates/server/tests/api.rs` (the `tower::oneshot`
   harness — see `mem_conn()`/`app_no_db()`/`app_db()` helpers at the top of
   that file for how existing tests spin up an app).
5. If the route table you're changing is one CLAUDE.md documents (the
   optimizer/instructor/members/enrollment tables), update that table too — and
   while you're there, don't assume the existing table is current; this repo
   has at least one known-stale doc claim (see the callout in §1).

### c. DB schema or migration change

`SCHEMA_SQL` (`rust/crates/db/src/schema.rs:11`) is **never edited in place**
for an existing installation — it's byte-compatible with the original
DB and with every already-deployed instance. New columns/tables are added as a
new, additive, idempotent migration function (see `add_admin_column`,
`add_yjs_state_column`, `add_eval_tables`, etc. in the same file for the
pattern — `ALTER TABLE ... ADD COLUMN`, swallowing the "duplicate column"
error, or `CREATE TABLE IF NOT EXISTS`), appended to `run_migrations()`. Query
logic lives in `rust/crates/db/src/helpers/` (one file per domain); note the `DbError` split
(`Sqlite | Missing(String) | Invalid(String)`) — `Missing` (a referenced row
is absent) and `Invalid` (rejected input or a conflict) are mapped to 4xx by the
server, so pick the right variant rather than always reaching for one. Extend `rust/crates/db/tests/helpers_parity.rs`
for the new behavior.

### d. Solver / constraint change

The actual optimization model lives in C++
(`rust/crates/solver/cpp/cpsat_shim.cpp`), not Rust — stage boundaries, the
`adj` hard constraint, and the two performance accelerators (prefixing,
pair-pruning) are all there. If you change the FFI function's parameters or
outputs, update **both sides of the boundary in the same change**: the
`extern "C"` signature in the `.cpp` file and the matching
`extern "C" { fn bs_cpsat_solve }` declaration in `solver/src/lib.rs:14` — a
mismatch here is a silent ABI bug, not a compile error. Changing the objective
or constraints will change the optimal objective values, so update the two
"golden" exact-optimum assertions in `solver/tests/solver_parity.rs`
deliberately (they're pinned on purpose — a green test suite after a
constraint change with unchanged golden values usually means the change didn't
do what you think). The parity bar throughout that test file is *structural
invariants + objective values*, not bit-identical assignments — CP-SAT with
`workers > 1` is nondeterministic about which optimum among ties it returns.

### e. Frontend change (`web/`)

`cd web && npm ci && npm run dev` serves the app with hot reload and proxies the
API and the collab socket to a server on :8000 (`vite.config.ts`). `npm run build`
is the gate (type-check + bundle); CI and the Docker build run it. Server calls go
through `api()` in `web/src/api.ts` — don't call `fetch` directly (it handles
errors and session expiry). Anything that changes boat assignments goes through
the Yjs hook (`useYSchedule`), never `POST /swap` — the doc is what `/commit`
persists. UI text is Dutch — match the existing tone/vocabulary and put strings
shared between components in `i18n.ts`. To see a frontend change served by the
real binary, `npm run build` and then rebuild the server (the bundle is embedded
at compile time).

### f. Auth / authorization change

Decide first whether this is a decision-logic change or a plumbing change.
Decision logic (who is allowed to do what, and why) belongs in
`core::authz` and is verified — if you're changing what `can_perform` or
`guard_set_admin` actually decide, that's a domain-rule change: follow §2a
(spec first). Plumbing (how the server *resolves* the inputs to that decision
— session lookup, the DB `is_admin` column, cookie handling, login throttling)
lives in `rust/crates/auth` and stays effectful; it should keep delegating to
`core::authz` rather than re-implementing a decision inline. New admin-gated
server actions should extend the `AdminCtx`/`CollabRole` capability pattern
(thread a capability value through), not add an ad-hoc boolean check at the
call site.

### g. CI/CD change

Three jobs, `.github/workflows/deploy.yml`: `test` (blocking, gates every PR
and push), `verify-spec` (additive today — recommended to make it a required
branch-protection check, but it doesn't currently block `deploy`), and
`deploy` (`needs: test`, only fires on push to `main`). **Never remove or
weaken an existing gate without explicit permission** — this is an
explicit repo rule (`CLAUDE.md`), not just good practice, because the whole
point of the spec/CI investment is that future changes (including
agent-authored ones) can be trusted *because* the gates can't be quietly
loosened. The actual production cutover is `deploy/deploy.sh`, run over SSH by
the `deploy` job: backup the live DB → build (or pull) the image → stand up a
throwaway copy on `:8001` against a *copy* of the DB and health/login-check it
→ hand over the real `:8000` service → health-gate it → automatically roll
back to the previous image on failure. Full detail (including one-time box
provisioning) is in `deploy/README.md` — this document only needs you to know
that the shape exists and where it lives.

---

## 3. Contribution mechanics

Feature work happens on short-lived, kebab-case, task-descriptive branches
(`rust-rewrite`, `ci-restore-test-job`, `aeneas`, ...) — not ticket-numbered.
Commits use a light `type(scope): summary` prefix (`spec(extract): ...`,
`refactor(core,spec): ...`, `ci+docs: ...`) rather than strict Conventional
Commits; it's a convention to match, not a linted rule. Merges into `main` go
through a PR (GitHub), and long-lived branches periodically merge
`main` back into themselves to stay in sync rather than rebasing.

| Gate | Blocking on PR/push? | Blocking for deploy? |
|---|---|---|
| `cargo test --workspace` (`test` job) | Yes | Yes (`deploy: needs: test`) |
| `cd spec && lake build` (`verify-spec` job) | Not yet enforced — recommended to make required | No |
| The on-box `deploy.sh` re-run of `cargo test` | N/A | Yes (build step of the cutover) |
| The on-box validate-on-`:8001` health/login check | N/A | Yes (cutover aborts on failure) |

---

## 4. Quick reference

```bash
# Rust: full test suite (the CI gate)
cd rust && cargo test --workspace

# Rust: one test
cd rust && cargo test -p boatsched-solver solver_parity

# Run the server natively
cd rust && cargo run -p boatsched-server -- --db boatsched.db

# Lean spec: build + proof-check (both tiers)
cd spec && lake build

# Regenerate Tier-1 conformance vectors after changing spec/BoatschedSpec
cd spec && lake exe gen-vectors > ../rust/crates/core/tests/conformance_vectors.json

# Re-extract the Tier-2 Aeneas model after changing validate/authz logic
# (exact pinned command: see spec/README.md "Regenerating the extracted model")

# Production image (build context is the REPO ROOT — builds web/ and embeds it)
docker build -t boatsched -f rust/Dockerfile .
```

| Doc | Owns |
|---|---|
| [`README.md`](../README.md) | User-facing overview, local dev, env vars, CLI flags |
| [`CLAUDE.md`](../CLAUDE.md) | Terse reference for coding agents: route tables, DB schema, constraints |
| [`spec/README.md`](../spec/README.md) | Lean spec conventions, tier binding, regeneration commands, open proof obligations |
| [`deploy/README.md`](../deploy/README.md) | Authoritative deploy runbook: provisioning, secrets, rollback |
| This document | The map between all of the above, and the workflow for making a change |
