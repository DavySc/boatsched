# boatsched

Assign kids to boats over several days under level constraints, optimizing a
strict **lexicographic** objective:

1. **A** — on *restricted* days (default Sat/Sun/Tue/Wed) every boat must be
   level-homogeneous or span only adjacent levels; **minimize mixed boats**.
2. **B** — on *diverse* days (default Fri/Mon) **maximize distinct levels** per boat.
3. **C** — across all days **minimize repeat co-boatings** (Social-Golfer style).

Built on OR-Tools CP-SAT. Levels are ordered low→high (default `k < b < A < B`).

## Install

```bash
python -m venv .venv
source .venv/bin/activate          # Windows: .venv\Scripts\activate
pip install -e .                   # add [dev] for tests: pip install -e ".[dev]"
```

This installs two commands: `boatsched` (CLI) and `boatsched-server` (web app).

## CLI

```bash
# from a roster CSV (columns: name,level)
boatsched --roster examples/roster_52.csv --boats 13 --out schedule.csv

# or a synthetic roster from per-level counts
boatsched --counts k=12 b=16 A=12 B=12 --boats 13
```

## Web app

```bash
boatsched-server                   # http://localhost:8000
```

Open the URL, drop a roster CSV, wait for the solve, and the schedule renders
(boats grouped by level; also a per-person companions view). Runs locally; no
data leaves the machine.

## Configuration

All knobs are CLI flags on both commands and propagate to the web UI:

| flag | meaning | default |
|---|---|---|
| `--boats` | number of boats (kids must divide evenly) | `13` |
| `--levels` | levels low→high | `k b A B` |
| `--adj` | max level-index spread on restricted days | `1` |
| `--restricted` | restricted day names | `Sat Sun Tue Wed` |
| `--diverse` | diverse day names | `Fri Mon` |
| `--stage-seconds` | budget for stages A and B each | `45` |
| `--seconds` | budget for stage C | `60` |
| `--workers` | parallel search workers | `8` |

## Library

```python
from boatsched import Config, solve, load_roster

kids = load_roster("examples/roster_52.csv")
rows, stats = solve(kids, Config(boats=13))
# rows: list of (day, boat, name, level); stats: {mixed, distinct, repeats, status_C}
```

## Notes

- Stages A and B prove optimality but need time; if A reports a nonzero mixed
  count on a year whose level counts are all multiples of the boat capacity,
  raise `--stage-seconds` — that's a time-budget issue, not arithmetic.
- Stage C usually finishes `FEASIBLE` (good, not proven minimal); more
  `--seconds` tightens it.
- The server is a local single-user tool (in-memory jobs). Making it
  multi-user or public-facing needs a real WSGI server and hardening.

## Tests

```bash
pip install -e ".[dev]"
pytest
```
