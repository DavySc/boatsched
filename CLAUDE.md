# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

The application is a **Rust** server (Axum) serving a single embedded SPA. The
`spec/` directory is the **Lean 4 formal model of record** backing the Rust
implementation (see "Formal domain spec" below).

## Repository layout

```
boatsched/
├── rust/                          Rust workspace — the production artifact
│   ├── crates/core                domain types + invariants (EnrollmentLevel, Config, validate)
│   ├── crates/roster              roster CSV ingestion
│   ├── crates/db                  SQLite access (rusqlite) + idempotent migrations
│   ├── crates/solver              OR-Tools CP-SAT model via C++ FFI shim
│   ├── crates/auth                sessions, scrypt passwords, rate-limiting
│   ├── crates/server              Axum HTTP + WebSocket app (binary: boatsched-server)
│   ├── Dockerfile                 multi-stage image (binary + pinned OR-Tools SDK)
│   └── docker-entrypoint.sh       forwards SOLVER_WORKERS → --workers
├── src/boatsched/
│   └── viewer.html                Unified SPA (optimizer + eval tracker) — embedded
│                                  into the server binary at compile time
├── spec/                          Lean 4 formal model of record (backs Rust)
├── examples/roster_52.csv         sample roster (used by the solver benchmark)
├── deploy/
│   ├── nginx.conf
│   ├── boatsched-docker.service   prod: Docker container via compose
│   └── deploy.sh                  never-break-prod cutover (validate + auto-rollback)
└── docker-compose.yml             prod Rust stack (+ docker-compose.validate.yml)
```

## Setup & commands

```bash
# Run the full test suite (this is the CI gate)
cd rust && cargo test --workspace

# Run a single test
cd rust && cargo test -p boatsched-solver solver_parity

# Run the server natively (serves on :8000)
cd rust && cargo run -p boatsched-server -- --db boatsched.db

# Build the production image (build context is the REPO ROOT — see below)
docker build -t boatsched -f rust/Dockerfile .

# Verify the Lean spec (a green build IS the proof check)
cd spec && lake build
```

## Architecture

### Rust workspace (`rust/crates/`)

- **`core`** — pure domain core: `EnrollmentLevel` (`k`<`b`<`A`<`B`), `Config`
  (all solver tunables + day sets), and `validate` (the roster/config failure
  taxonomy). This is the code bound to the Lean spec via conformance vectors.
- **`roster`** — roster ingestion (file + raw bytes for HTTP uploads + synthetic
  from counts).
- **`solver`** — the CP-SAT model. Runs stages A→B→C in strict lexicographic
  order, locking each optimum as a hard constraint before the next stage. Calls
  OR-Tools' C++ engine through an FFI shim (`crates/solver/cpp/`); `build.rs`
  bakes an rpath to the bundled SDK.
- **`db`** — all SQLite access (rusqlite). Schema is created with
  `CREATE TABLE IF NOT EXISTS`; migrations are additive and idempotent. The
  schema is **byte-compatible** with the legacy Python DB (reused in place).
- **`auth`** — in-memory session store, scrypt password hashing (byte-compatible
  with the old hashes), and the per-IP failed-login throttle.
- **`server`** — the Axum app factory + the binary `boatsched-server`. Serves
  `viewer.html` at `/` (embedded via `include_str!`), exposes the JSON/WS API,
  and runs solve jobs on `spawn_blocking`. The binary parses CLI flags (mirroring
  the documented `boatsched-server` flags) and, whenever `--db` is passed, flips
  every security knob to its strict production value. See **Authentication &
  hardening** below.

> **Build inputs outside `rust/`.** The Docker build context is the **repo root**
> because the server binary embeds two files that live outside `rust/`:
> `src/boatsched/viewer.html` (`include_str!`) and — for the solver benchmark —
> `examples/roster_52.csv`. The Dockerfile `COPY`s `viewer.html` into the image
> so the relative `include_str!` path resolves. Everything else the image needs
> is under `rust/`.

### Server routes

#### Optimizer / scheduling

| Method | Path | Description | Admin? |
|--------|------|-------------|--------|
| GET | `/` , `/index.html` | Serve `viewer.html` | public |
| GET | `/config` | Solver config JSON | auth |
| GET/POST | `/seasons` | List / create seasons | POST: admin |
| GET | `/boats` , POST `/boat` | List boats / rename a boat | auth |
| GET | `/schedules?season_id=` | List schedules for a season | auth |
| GET | `/schedule?id=` | Get schedule rows + start date | auth |
| POST | `/solve?season_id=` | Start background solve job | admin (with DB) |
| GET | `/status?id=` | Poll solve job | auth |
| POST | `/swap` | Swap two kids between boats | admin |
| WS | `/collab/{schedule_id}` | Yjs collaborative editing (non-admins observe only) | auth |
| POST | `/schedule/{id}/commit` | Validate and write Yjs state to DB | admin |
| POST | `/schedule/{id}/start-date` | Set schedule start date | auth |

> **Boat-assignment editing is admin-only at all times.** `/swap`, `/commit`
> and WS update frames require an admin (`{"detail": "Only admins may edit boat
> assignments"}`, 403, for non-admins). Admins are **not** blocked by the
> start-date lock; the lock now only drives the read-only UI badge, not server
> enforcement. Non-admins may still open a `/collab` socket to watch live edits,
> but any update frame they send is dropped.

#### Instructor / auth

| Method | Path | Description | Admin? |
|--------|------|-------------|--------|
| GET | `/instructors` | List instructors (`is_admin`, `has_password`) | auth |
| POST | `/instructor` | Create instructor (first one auto-admin) | admin |
| POST | `/set-password` | Change a password (own, or any if admin) | self/admin |
| POST | `/api/auth/login` | JSON login `{name, password}` → `bsession` cookie | public |
| POST | `/api/auth/logout` | Clear session | public |
| GET | `/api/auth/me` | `{member_id, name, is_admin}` for current session | auth |
| POST | `/api/instructors/{member_id}/set-admin` | Grant/revoke admin (`{is_admin}`) | admin |
| GET | `/api/health` | Health check (no auth — for uptime monitors) | public |

#### Members (JSON, called by viewer.html)

| Method | Path | Description | Admin? |
|--------|------|-------------|--------|
| GET | `/api/members` | List active members `[{id, name}]` | auth |
| GET | `/api/members/progression` | `{seasons, members[]}` — level per season, excludes instructors | auth |
| PUT | `/api/members/{id}` | Rename a member `{name}` | admin |
| DELETE | `/api/members/{id}` | Soft-delete (`active=0`); 409 if member is an instructor | admin |
| POST | `/api/members/import` | Seed members from a CSV with a `name` column | admin |

#### Enrollment / student levels (JSON / CSV)

> `{enrollment_id}` is the **season id** — enrollments are keyed by
> `(member_id, season_id)`; there is no separate `enrollment_id` column.

| Method | Path | Description | Admin? |
|--------|------|-------------|--------|
| PATCH | `/enrollment/{enrollment_id}/member/{member_id}/level` | Set a student's level for the season (`{"level": "k\|b\|A\|B"}`); 400 on invalid level, 404 if not an enrolled non-instructor student | admin |
| GET | `/enrollment/{enrollment_id}/students/export` | CSV of non-instructor members (`name,level`) | admin |

> Student levels are **always admin-only**, independent of the schedule lock.
> Levels reuse the existing `enrollment.level` enum (`'k'\|'b'\|'A'\|'B'`,
> enforced by the table CHECK and tied to the solver ordering) — not free text.

> **Known gap:** `viewer.html`'s evaluation cards call `GET`/`POST /api/progress`,
> but no such route exists in `server.py` — those calls fail silently (wrapped
> in `try`/`catch`). Skill-result persistence (`skill_result` / `v_level_card`)
> is modelled in the DB but not yet wired to an API.

### DB schema

The full schema below is created by `_create_tables()` (do not modify — see
constraints). The migration helpers add columns to **pre-existing** databases
made by older versions; on a fresh DB they are no-ops.

```sql
member          (id, name, active, joined)             -- NO level column
instructor      (instructor_id, member_id, password_hash BLOB,
                 password_salt BLOB, is_admin)          -- is_admin via migration
season          (id, year)
schedule        (id, season_id, created, start_date, config TEXT,
                 yjs_state BLOB)                         -- yjs_state via migration
boat            (boat_id, name, notes)                  -- seeds boat_id 1..13
boat_assignment (id, schedule_id, member_id, boat_id, day)
enrollment      (member_id, season_id, level CHECK IN ('k','b','A','B'),
                 PRIMARY KEY (member_id, season_id))     -- level is PER SEASON
skill           (id, level, code, label, ordinal, retired)
skill_result    (member_id, season_id, skill_id, status, note,
                 FK (member_id, season_id) → enrollment)
v_level_card    VIEW joining enrollment × skill (live skills) ⟕ skill_result
```

Migrations, all idempotent, run once at startup: add the `yjs_state`,
`start_date` and `is_admin` columns; add the enrollment tables; drop the legacy
`member.level` column; drop the old `progress_record` / `day_allocation` camp
tables; and promote all existing instructors to admin the first time admin roles
appear. They are no-ops on a fresh DB and safe to re-run on the byte-compatible
legacy DB.

#### Levels & membership

- A member's level is **per season**, stored as a string `'k' | 'b' | 'A' | 'B'`
  in `enrollment` (low→high). There is no `member.level` column and no numeric
  mapping helper anymore.
- viewer.html maps the codes to Dutch labels via `LV_LABEL`
  (`k`=Ketelbinkie, `b`=Bemanning, `A`=A, `B`=B) and colours via `LV_COLOR`.
- **Roster CSV semantics** (member import): `'k'`-level rows are new
  participants and are auto-created in `member`; every other level must already
  exist in `member` or an error is raised (prevents silently dropping returning
  members from a solve).

### Lexicographic objective (A > B > C)

| Stage | Objective | Days |
|---|---|---|
| A | Minimise mixed-level boats | `restricted` (Sat/Sun/Tue/Wed by default) |
| B | Maximise distinct levels per boat | `diverse` (Fri/Mon by default) |
| C | Minimise repeat co-boatings (Social-Golfer) | all days |

`adj` (default 1) is a hard constraint: on restricted days boats may only span levels whose indices differ by at most `adj`.

### Yjs collaborative editing

`/collab/{schedule_id}` implements the y-websocket binary sync protocol (lib0 varint encoding). The server maintains one `yrs` doc per schedule (Yjs v1 encoding, wire-compatible with the old pycrdt state), lazily loaded from the `yjs_state` BLOB. The Y.Doc model:

- Map `"schedule"` keyed `"{day}:{boat}"` (e.g. `"Sat:3"`)
- Each value is a Y.Array of member name strings

On solve completion the job initialises and persists the Yjs state. "Commit schedule" (`POST /schedule/{id}/commit`) validates the doc and writes to `boat_assignment`.

### Authentication & hardening

All non-public routes require a `bsession` session cookie (in-memory store,
`(member_id, expiry)` with TTL eviction). Modes:

- **Open mode** (dev only) — no DB, or no instructor has a password yet. All
  requests are allowed. **Disabled when `require_auth=True`.**
- **Auth mode** — at least one password is set (or `require_auth=True`). Every
  protected route needs a valid session.

**Roles.** `instructor.is_admin` gates privileged actions: run the solver,
create seasons, import/rename/delete members, create instructors, and grant or
revoke admin. Non-admins may log in, view schedules, edit collaboratively, and
change their own password. In open mode everyone is treated as admin.

**Production bootstrap.** There is no anonymous window: the server seeds the
first admin from `BOATSCHED_ADMIN` / `BOATSCHED_ADMIN_PW` at startup
(idempotent — sets the password only if none exists, so UI changes survive
restarts).

**Hardening (enabled when `--db` is set; see env-var table in `README.md`):**
`require_auth`, `secure_cookies`, session TTL, per-IP failed-login throttle
(429), upload size caps (413), WS frame-size guard, job pruning, generic 500
responses, security headers in middleware, and a `min_password_len` of 8. The
`crates/auth` and `crates/server` test modules cover this; the deploy configs
(`deploy/nginx.conf` TLS + rate limits) complete the picture.

### Solver output

The solver returns the assignment rows `(day, boat, name, level)` plus stats
`{mixed, distinct, repeats, status_C}` (matching the lexicographic stages
below). A progress callback streams per-stage results for the live UI.

### Error categories

- **validation** — roster/config structurally infeasible
- **solve** — a stage found no solution within its time budget
- **roster** — malformed CSV input

## Constraints: what NOT to modify

- **The Lean spec (`spec/`) is the source of truth for the pure domain core.**
  Before changing domain behaviour in `rust/crates/core` (levels/ordering,
  `Config`, `validate`, the authorization gate), update the matching rule in
  `spec/` in the same change and keep `cargo test`'s conformance check green.
- **Do not change the DB schema shape** — it is byte-compatible with the live
  production DB. Migrations must stay additive and idempotent.
- **Never remove or weaken a test/CI gate** (`cargo test --workspace`, the
  conformance vectors, the on-box build gate) without explicit permission.

## Formal domain spec (Lean 4)

`spec/` is the **formal model of record** for the domain — a Lean 4 library that
defines the core types and *proves* the invariants the Rust code must uphold
(levels & ordering, `Config.days`, `validate`'s failure taxonomy + happy-path
soundness, the authorization gate / no-privilege-escalation). It backs the Rust
implementation to make agentic changes safer.

- **Before changing domain behaviour in `rust/`, read the matching rule in
  `spec/` and update it in the same change.** The spec is the source of truth.
- Verify with `cd spec && lake build` — a green build *is* the proof check.
- Each theorem has a stable ID (`LVL-*`, `CFG-*`, `VAL-*`, `AUTH-*`, `EXT-*`); the
  ID↔Rust traceability table and the Tier-1/Tier-2 binding model are in
  `spec/README.md`.
- Scope: only the pure core is formalized (FFI/axum/tokio/yrs are out of reach).
  **Two binding tiers:**
  - **Tier 1 — conformance vectors** (all of `core`): the proven oracle
    (`spec/BoatschedSpec/Conformance.lean`) is emitted to
    `rust/crates/core/tests/conformance_vectors.json` (inside the Docker build
    context, so the image `cargo test` gate runs it) → checked by
    `rust/crates/core/tests/conformance.rs` (regen with `cd spec && lake exe
    gen-vectors > ../rust/crates/core/tests/conformance_vectors.json`).
  - **Tier 2 — Charon+Aeneas refinement** (`validate` + `authz` only): the
    *real* Rust functions are extracted to `spec/BoatschedExtract/` and **proven
    to refine the spec for all inputs, panic-free** (`EXT-VAL`, `EXT-AUTH`). This
    is why `validate`/`ProblemConfig`/`authz` are written in an extraction-friendly
    style (index loops, no iterators/`HashSet`/`format!`, float-free
    `ProblemConfig`) — **keep them that way**, and if you change a verified
    function, re-extract the model (see `spec/README.md`) and keep `lake build`
    green. Everything Tier-2 covers is also covered by Tier 1.
- The spec depends on **Mathlib** + (for Tier 2) **Aeneas**; `cd spec && lake
  build` is gated in CI by the `verify-spec` job. Prefer trusted libraries
  (Mathlib, `Lean.Json`, `serde`) over hand-rolled code; no `sorry`.

## Deployment

Production runs the **Rust** server as a **single Docker container**
(`rust/Dockerfile`, multi-stage `ubuntu:24.04`) on a Hetzner box, behind
nginx+certbot, managed by `deploy/boatsched-docker.service`. The image bundles a
version-pinned OR-Tools SDK (9.15.6755, `ubuntu-24.04` build) so it is
self-contained — the host only needs Docker. The SQLite DB is bind-mounted from
`/var/lib/boatsched` and reused unchanged (the Rust schema/migrations are
byte-compatible with the Python DB).

CI/CD (`.forgejo/workflows/deploy.yml`), all on the Codeberg **shared runner**
(`codeberg-small`): a **`test` job** runs `cargo test --workspace` on every PR and
every push to `main`; the **`deploy` job** runs only on a push to `main` and only
if tests pass (`needs: test`). Deploy SSHes to the box → `deploy/deploy.sh`, which
**builds the image on the box** (`BUILD_ON_HOST=1`; Codeberg CI can't build OCI
images and won't accept self-hosted runners — so no registry). The on-box build
runs `cargo test` again (gate enforced twice). The cutover is
**never-break-prod**: backup DB → build → validate the image against a *copy* on
`:8001` → single-writer handover on `:8000` → health-gate with **automatic
rollback** to the previous image. **Never remove a test/CI gate without explicit
permission.** See `deploy/README.md` for the full runbook.

Quick local run (build context is the repo root — the binary embeds
`src/boatsched/viewer.html`):

```bash
docker build -t boatsched -f rust/Dockerfile .   # build the image
docker compose up -d                   # run (needs /etc/boatsched/env + DB dir)
```
