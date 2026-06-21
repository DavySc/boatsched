# CLAUDE.md
claude --resume 3f74bae1-11a5-49f1-b98d-cb61742a354b
This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository layout

```
boatsched/
├── src/boatsched/     Python package
│   ├── solver.py      CP-SAT model — DO NOT MODIFY
│   ├── roster.py      roster ingestion — DO NOT MODIFY
│   ├── cli.py         CLI entry point — DO NOT MODIFY
│   ├── db.py          SQLite helpers
│   ├── server.py      FastAPI app factory
│   └── viewer.html    Unified SPA (optimizer + eval tracker, embedded asset)
├── tests/
├── rust/                   Rust rewrite (workspace; prod artifact)
│   └── Dockerfile          multi-stage image (binary + OR-Tools SDK)
├── deploy/
│   ├── nginx.conf
│   ├── boatsched.service          legacy Python systemd unit (rollback target)
│   ├── boatsched-docker.service   prod: Docker container via compose
│   └── deploy.sh                  never-break-prod cutover (validate + auto-rollback)
├── Dockerfile              legacy Python image
├── docker-compose.yml      prod Rust stack (+ docker-compose.validate.yml)
└── pyproject.toml
```

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"    # includes pytest, httpx, pytest-asyncio
```

## Commands

```bash
# Run all tests
pytest

# Run a single test
pytest tests/test_boatsched.py::test_tiny_solve

# Python backend (serves on :8000)
boatsched-server --db boatsched.db

# CLI
boatsched --roster examples/roster_52.csv --boats 13 --out schedule.csv
boatsched --counts k=12 b=16 A=12 B=12 --boats 13
```

## Architecture

### Python package (`src/boatsched/`)

- **`solver.py`** — core CP-SAT model. `Config` holds all tunables. `staged_solve()` runs stages A→B→C in strict lexicographic order, locking each optimum as a hard constraint before the next stage. `solve()` is the single entry point used by callers and tests.
- **`roster.py`** — roster ingestion. `load_roster()` reads a file; `parse_roster_bytes()` reads raw bytes for HTTP uploads; `roster_from_counts()` builds a synthetic roster.
- **`server.py`** — FastAPI app factory. Background solve jobs run in `threading.Thread(daemon=True)`. Serves `viewer.html` at `/`. Signature:
  ```python
  create_app(cfg, db_conn=None, *,
             require_auth=False, secure_cookies=False, session_ttl=12*3600,
             login_max_failures=20, login_window=300,
             max_upload_bytes=5*1024*1024, max_ws_message_bytes=1024*1024,
             min_password_len=1) -> FastAPI
  ```
  Every security knob defaults to the permissive/dev value so the test suite is
  unaffected; `main()` flips them to strict production values whenever `--db` is
  passed. See **Authentication & hardening** below.
- **`cli.py`** — thin argparse wrapper around `solve()`.
- **`db.py`** — all SQLite helpers. Schema is created by `_create_tables()`; migrations are additive ALTER TABLE calls.

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

Migration functions, all idempotent, run once in `create_app()`:
`add_yjs_state_column`, `add_start_date_column`, `add_enrollment_tables`,
`drop_member_level_column` (removes legacy `member.level`),
`drop_legacy_camp_tables` (drops old `progress_record` / `day_allocation`),
`add_admin_column`, `auto_grant_first_admin` (promotes all existing instructors
the first time admin roles appear).

#### Levels & membership

- A member's level is **per season**, stored as a string `'k' | 'b' | 'A' | 'B'`
  in `enrollment` (low→high). There is no `member.level` column and no numeric
  mapping helper anymore.
- viewer.html maps the codes to Dutch labels via `LV_LABEL`
  (`k`=Ketelbinkie, `b`=Bemanning, `A`=A, `B`=B) and colours via `LV_COLOR`.
- **Roster CSV semantics** (`add_or_update_members`): `'k'`-level rows are new
  participants and are auto-created in `member`; every other level must already
  exist in `member` or a `RuntimeError` is raised (prevents silently dropping
  returning members from a solve).

### Lexicographic objective (A > B > C)

| Stage | Objective | Days |
|---|---|---|
| A | Minimise mixed-level boats | `restricted` (Sat/Sun/Tue/Wed by default) |
| B | Maximise distinct levels per boat | `diverse` (Fri/Mon by default) |
| C | Minimise repeat co-boatings (Social-Golfer) | all days |

`adj` (default 1) is a hard constraint: on restricted days boats may only span levels whose indices differ by at most `adj`.

### Yjs collaborative editing

`/collab/{schedule_id}` implements the y-websocket binary sync protocol (lib0 varint encoding). The server maintains one `pycrdt.Doc` per schedule, lazily loaded from the `yjs_state` BLOB. The Y.Doc model:

- Map `"schedule"` keyed `"{day}:{boat}"` (e.g. `"Sat:3"`)
- Each value is a Y.Array of member name strings

On solve completion `_run_job` initialises and persists the Yjs state. "Commit schedule" (`POST /schedule/{id}/commit`) validates the doc and writes to `boat_assignment`.

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

**Production bootstrap.** There is no anonymous window: `main()` seeds the first
admin from `BOATSCHED_ADMIN` / `BOATSCHED_ADMIN_PW` via `db.ensure_admin()`
(idempotent — sets the password only if none exists, so UI changes survive
restarts).

**Hardening (enabled by `main()` when `--db` is set; see env-var table in
`README.md`):** `require_auth`, `secure_cookies`, session TTL, per-IP failed-
login throttle (429), upload size caps (413), WS frame-size guard, `JOBS`
pruning, generic 500 responses (`_server_error`), security headers in
middleware, and a `min_password_len` of 8. Tests live in
`tests/test_security.py`; the deploy configs (`deploy/nginx.conf` TLS + rate
limits, `deploy/boatsched.service` env-file + sandboxing) complete the picture.

### Public Python API

```python
from boatsched import Config, solve, load_roster

kids = load_roster("examples/roster_52.csv")
rows, stats = solve(kids, Config(boats=13))
# rows: List[Tuple[day, boat_number, name, level]]
# stats: {"mixed": int, "distinct": int, "repeats": int, "status_C": str}
```

`on_progress` is an optional `(label, status_name, value) -> None` callback for streaming stage results.

### Error types

- `ValidationError` — roster/config structurally infeasible
- `SolveError` — a stage found no solution within its time budget
- `RosterError` — malformed CSV input

## Constraints: what NOT to modify

- `solver.py`, `roster.py`, `cli.py` — do not change
- `db._create_tables()` — do not change; migrations are additive ALTER TABLE calls
- Existing `db.py` function signatures — may be extended but not renamed or changed

## Formal domain spec (Lean 4)

`spec/` is the **formal model of record** for the domain — a Lean 4 library that
defines the core types and *proves* the invariants the Rust code must uphold
(levels & ordering, `Config.days`, `validate`'s failure taxonomy + happy-path
soundness, the authorization gate / no-privilege-escalation). It backs the Rust
implementation to make agentic changes safer.

- **Before changing domain behaviour in `rust/`, read the matching rule in
  `spec/` and update it in the same change.** The spec is the source of truth.
- Verify with `cd spec && lake build` — a green build *is* the proof check.
- Each theorem has a stable ID (`LVL-*`, `CFG-*`, `VAL-*`, `AUTH-*`); the
  ID↔Rust traceability table and the Tier-1/Tier-2 binding model are in
  `spec/README.md`.
- Scope: only the pure core is formalized (FFI/axum/tokio/yrs are out of reach).
  Rust is bound to the proven model via conformance vectors
  (`spec/BoatschedSpec/Conformance.lean`) emitted to
  `rust/crates/core/tests/conformance_vectors.json` (inside the Docker build
  context, so the image `cargo test` gate runs it) → checked by
  `rust/crates/core/tests/conformance.rs` (regen with `cd spec && lake exe
  gen-vectors > ../rust/crates/core/tests/conformance_vectors.json`).
- The spec depends on **Mathlib** (one-time `cd spec && lake exe cache get`).
  Prefer trusted libraries (Mathlib, `Lean.Json`, `serde`) over hand-rolled
  code; no `sorry`.

## Deployment

Production runs the **Rust** server as a **single Docker container**
(`rust/Dockerfile`, multi-stage `ubuntu:24.04`) on a Hetzner box, behind
nginx+certbot, managed by `deploy/boatsched-docker.service`. The image bundles a
version-pinned OR-Tools SDK (9.15.6755, `ubuntu-24.04` build) so it is
self-contained — the host only needs Docker. The SQLite DB is bind-mounted from
`/var/lib/boatsched` and reused unchanged (the Rust schema/migrations are
byte-compatible with the Python DB).

CD (`.forgejo/workflows/deploy.yml`): push to `main` → a Codeberg **shared
runner** (`codeberg-small`) SSHes to the box → `deploy/deploy.sh`, which **builds
the image on the box** (`BUILD_ON_HOST=1`; Codeberg CI can't build OCI images and
won't accept self-hosted runners — so no registry). The build runs `cargo test`
as the gate. The cutover is **never-break-prod**: backup DB → build → validate the
image against a *copy* on `:8001` → single-writer handover on `:8000` →
health-gate with **automatic rollback**. See `deploy/README.md` for the full
runbook and `deploy/CUTOVER_CHECKLIST.md` for the live-migration steps.

Quick local run (build context is the repo root — the binary embeds
`src/boatsched/viewer.html`):

```bash
docker build -t boatsched -f rust/Dockerfile .   # build the image
docker compose up -d                   # run (needs /etc/boatsched/env + DB dir)
```
