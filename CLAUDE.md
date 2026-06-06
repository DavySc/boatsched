# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"    # includes pytest
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
```

## Architecture

The package lives under `src/boatsched/` and has three modules:

- **`solver.py`** — core CP-SAT model and solve pipeline. `Config` holds all tunables. `build_model()` constructs the OR-Tools model with three objective variables (`total_mixed`, `total_distinct`, `total_repeats`). `staged_solve()` runs them in strict lexicographic order A→B→C, locking each optimal value as a hard constraint before solving the next stage. `solve()` is the single end-to-end entry point that callers and tests use.
- **`roster.py`** — roster ingestion only. `load_roster()` reads a file; `parse_roster_bytes()` reads raw bytes (used by the server for HTTP uploads); `roster_from_counts()` builds a synthetic roster from a `{level: n}` dict.
- **`server.py`** — stdlib-only HTTP server (no framework). `POST /solve` accepts a roster CSV (multipart or raw body), spawns a daemon thread that calls `solve()`, and stores the result in the in-memory `JOBS` dict. `GET /status?id=` polls it. The UI is the single embedded `viewer.html` file (served as a package data resource).
- **`cli.py`** — thin argparse wrapper around `solve()`. Prints progress to stderr, results to stdout, and optionally writes a CSV.

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
