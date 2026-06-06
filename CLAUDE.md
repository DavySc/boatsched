# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"    # includes pytest, httpx, pytest-asyncio
```

## Commands

```bash
# Run tests
pytest

# Run a single test
pytest tests/test_boatsched.py::test_tiny_solve

# CLI (requires --roster or --counts, and --boats)
boatsched --roster examples/roster_52.csv --boats 13 --out schedule.csv
boatsched --counts k=12 b=16 A=12 B=12 --boats 13

# Web server
boatsched-server              # http://localhost:8000
boatsched-server --db boatsched.db   # with SQLite persistence
```

## Architecture

The package lives under `src/boatsched/` and has four modules:

- **`solver.py`** — core CP-SAT model and solve pipeline. `Config` holds all tunables. `build_model()` constructs the OR-Tools model with three objective variables (`total_mixed`, `total_distinct`, `total_repeats`). `staged_solve()` runs them in strict lexicographic order A→B→C, locking each optimal value as a hard constraint before solving the next stage. `solve()` is the single end-to-end entry point that callers and tests use.
- **`roster.py`** — roster ingestion only. `load_roster()` reads a file; `parse_roster_bytes()` reads raw bytes (used by the server for HTTP uploads); `roster_from_counts()` builds a synthetic roster from a `{level: n}` dict.
- **`server.py`** — FastAPI/uvicorn HTTP+WebSocket server. `create_app(cfg, db_conn) → FastAPI` is the app factory used by tests and `main()`. Background solve jobs run in `threading.Thread(daemon=True)`. The UI is the single embedded `viewer.html` file.
- **`cli.py`** — thin argparse wrapper around `solve()`. Prints progress to stderr, results to stdout, and optionally writes a CSV.

### Server endpoints

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

### Lexicographic objective (A > B > C)

| Stage | Objective | Days |
|---|---|---|
| A | Minimize mixed-level boats | `restricted` (Sat/Sun/Tue/Wed by default) |
| B | Maximize distinct levels per boat | `diverse` (Fri/Mon by default) |
| C | Minimize repeat co-boatings (Social-Golfer) | all days |

`adj` (default 1) is a hard constraint on restricted days: boats may only contain levels whose indices differ by at most `adj`; wider gaps are forbidden.

### Key constraints

- Every kid is assigned to exactly one boat per day (partition constraint).
- Every boat has exactly `N // boats` kids per day (uniform capacity).
- Stage C is the hard one — it usually finishes `FEASIBLE`, not `OPTIMAL`. Raise `--seconds` to tighten it.

### Yjs collaborative editing

The WebSocket endpoint `/collab/{schedule_id}` implements the y-websocket binary sync protocol (lib0 varint encoding). The server maintains one `pycrdt.Doc` per schedule (lazily loaded from the `yjs_state` BLOB column on the `schedule` table). The Y.Doc data model:

- Map named `"schedule"` with keys `"{day}:{boat}"` (e.g. `"Sat:3"`)
- Each value is a Y.Array of member name strings

On solve completion, `_run_job` initializes and persists the Yjs state from the solver result. The browser connects via `WebsocketProvider` from `y-websocket` (imported from esm.sh CDN). "Commit schedule" calls `POST /schedule/{id}/commit` which validates the Yjs doc (no kid on two boats same day) and writes the canonical assignment to `boat_assignment`.

### DB schema additions

`db.py` has three Yjs-related functions added after the original set (do not move or rename existing functions):

- `add_yjs_state_column(conn)` — ALTER TABLE idempotent; called by `create_app`
- `get_yjs_state(conn, schedule_id)` → bytes | None
- `set_yjs_state(conn, schedule_id, state)` → None
- `replace_assignments(conn, schedule_id, assignments)` — DELETE + re-INSERT for commit

### Public API

```python
from boatsched import Config, solve, load_roster

kids = load_roster("examples/roster_52.csv")
rows, stats = solve(kids, Config(boats=13))
# rows: List[Tuple[day, boat_number, name, level]]
# stats: {"mixed": int, "distinct": int, "repeats": int, "status_C": str}
```

`on_progress` is an optional `(label, status_name, value) -> None` callback passed to `solve()` for streaming stage results.

### Error types

- `ValidationError` — roster/config structurally infeasible (bad divisibility, unknown levels, etc.)
- `SolveError` — a stage found no solution within its time budget
- `RosterError` — malformed CSV input

### Constraints: what NOT to modify

- `solver.py`, `roster.py`, `cli.py` — do not change
- `db._create_tables()` — do not change; migration is handled by `add_yjs_state_column`
- Existing `db.py` functions — may be extended but not renamed or have signatures changed
