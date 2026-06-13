# CLAUDE.md

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
├── deploy/
│   ├── nginx.conf
│   └── boatsched.service
├── Dockerfile
├── docker-compose.yml
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
- **`server.py`** — FastAPI app factory: `create_app(cfg, db_conn) → FastAPI`. Background solve jobs run in `threading.Thread(daemon=True)`. Serves `viewer.html` at `/`.
- **`cli.py`** — thin argparse wrapper around `solve()`.
- **`db.py`** — all SQLite helpers. Schema is created by `_create_tables()`; migrations are additive ALTER TABLE calls.

### Server routes

#### Optimizer

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | Serve `viewer.html` |
| GET | `/config` | Solver config JSON |
| GET/POST | `/seasons` | List / create seasons |
| GET/POST | `/boats`, `/boat` | List boats / rename a boat |
| GET | `/schedules?season_id=` | List schedules for a season |
| GET | `/schedule?id=` | Get schedule rows |
| POST | `/solve?season_id=` | Start background solve job |
| GET | `/status?id=` | Poll solve job |
| POST | `/swap` | Swap two kids between boats |
| WS | `/collab/{schedule_id}` | Yjs collaborative editing |
| POST | `/schedule/{id}/commit` | Validate and write Yjs state to DB |
| POST | `/schedule/{id}/start-date` | Set schedule start date (locks edits) |

#### Instructor management

| Method | Path | Description |
|--------|------|-------------|
| GET/POST | `/instructor` | Create additional instructor |
| POST | `/set-password` | Change instructor password |
| GET | `/instructors` | List instructors |

#### API (JSON, called by viewer.html)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/auth/login` | JSON login `{name, password}` → sets `bsession` cookie |
| POST | `/api/auth/logout` | Clear session |
| GET | `/api/auth/me` | Check current session |
| GET | `/api/health` | Health check |
| GET | `/api/students` | List active students `{students: Student[]}` |
| POST | `/api/students` | Upsert student list `{students}` |
| PUT | `/api/students/{id}` | Update one student |
| DELETE | `/api/students/{id}` | Soft-delete (sets active=0) |
| GET | `/api/progress` | All progress records `{progressData: StudentProgress[]}` |
| GET | `/api/progress/{studentId}` | One student's progress |
| POST | `/api/progress` | Save/update progress record |
| DELETE | `/api/progress/{studentId}` | Delete progress record |
| GET | `/api/boat-allocations` | All day allocations `{boatAllocations: DayAllocation[]}` |
| GET | `/api/boat-allocations/{day}` | One day's allocation |
| POST | `/api/boat-allocations` | Save/update day allocation |
| DELETE | `/api/boat-allocations/{day}` | Delete day allocation |

### DB schema

#### Original tables (do not modify `_create_tables()`)

```sql
member          (id, name, level TEXT CHECK IN ('k','b','A','B'), active, joined)
instructor      (instructor_id, member_id, password_hash BLOB, password_salt BLOB)
season          (id, year)
schedule        (id, season_id, created, start_date, config TEXT, yjs_state BLOB)
boat            (boat_id, name, notes)
boat_assignment (id, schedule_id, member_id, boat_id, day)
```

#### Camp tracker tables (added by `add_camp_tables()`)

```sql
progress_record (student_id TEXT PK, data_json TEXT)
day_allocation  (day TEXT PK, data_json TEXT)
```

`progress_record` and `day_allocation` store full JSON objects verbatim. `student_id` in `progress_record` is `member.id` serialised as a string.

#### Level mapping

viewer.html uses numeric levels; the DB uses strings:

| UI | DB |
|---|---|
| 1 (Ketelbinkie) | `'k'` |
| 2 (Bemanning) | `'b'` |
| 3 (A) | `'A'` |
| 4 (B) | `'B'` |

Mapping lives in `db.py`: `_LEVEL_TO_NUM`, `_NUM_TO_LEVEL`.

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

### Authentication

All non-public routes require a `bsession` session cookie. The server operates in two modes:

- **Open mode** — no instructor has a password yet. All requests are allowed.
- **Auth mode** — at least one instructor has a password. All routes require a valid session.

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

## Deployment

See `README.md` for full Docker and bare-metal Hetzner instructions.

Quick Docker deploy:

```bash
docker compose up --build -d
```

The `Dockerfile` packages everything into a Python 3.11 image. The database is persisted to the `boatsched_data` named volume at `/data/boatsched.db`.
