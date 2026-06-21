# boatsched

A sailing camp management application served as a single, mobile-friendly SPA
(`viewer.html`) backed by one Python server, one SQLite database, and one login.
It bundles:

- **Rooster Optimizer** — schedule generator that assigns kids to boats over
  multiple days under level constraints, optimising a strict lexicographic
  objective with OR-Tools CP-SAT, with real-time collaborative editing.
- **Member & instructor management** — seed members from CSV, manage admin
  roles, and view each member's level progression across seasons.
- **Evaluation cards** — per-camper daily skill-evaluation UI (skill-result
  persistence is modelled in the DB but not yet wired to an API — see CLAUDE.md).

The UI language is Dutch.

---

## Architecture overview

```
boatsched/
├── src/boatsched/     Python package (FastAPI server + solver + DB)
│   ├── server.py      FastAPI app factory + routes
│   ├── solver.py      OR-Tools CP-SAT model
│   ├── db.py          SQLite helpers + migrations
│   ├── roster.py      roster CSV ingestion
│   ├── cli.py         offline CLI entry point
│   └── viewer.html    the entire SPA (vanilla JS, embedded as a package asset)
├── tests/
├── deploy/            nginx + systemd config for bare-metal Hetzner
├── Dockerfile
├── docker-compose.yml
└── pyproject.toml
```

There is **no separate frontend build** — `viewer.html` is a self-contained
single-page app (it loads Yjs from esm.sh for collaborative editing). The Python
server serves it at `/` and exposes the JSON/WebSocket API it calls. See
`CLAUDE.md` for the full route and schema reference.

---

## Local development

### Prerequisites

- Python 3.11+

### Setup

```bash
python -m venv .venv
source .venv/bin/activate          # Windows: .venv\Scripts\activate
pip install -e ".[dev]"            # includes pytest, httpx, pytest-asyncio
```

### Running locally

```bash
source .venv/bin/activate
boatsched-server --db boatsched.db
# App at http://localhost:8000
```

With a fresh local database and no admin/password set, the server runs in **open
mode** — any login is accepted so you can start immediately. Set a password on an
instructor (or run with the production env vars below) to switch on real auth.

### Tests

```bash
pytest                                          # all tests with coverage
pytest tests/test_boatsched.py::test_tiny_solve # single test
```

---

## Deployment on Hetzner

> **Security model (read first).** In production the app refuses anonymous
> access: `--db` turns on `require_auth`, `secure_cookies`, an 8-char minimum
> password, and login rate-limiting. There is **no open/bootstrap window** —
> the first admin is seeded from environment variables, so you must set
> `BOATSCHED_ADMIN` and `BOATSCHED_ADMIN_PW` before first start or nobody can
> log in. Always run behind the provided nginx + TLS config; never expose port
> 8000 to the internet directly.

Production runs the **Rust** server as a single Docker container behind
nginx+certbot. **`deploy/README.md` is the authoritative runbook** (provisioning,
CI secrets, the never-break-prod cutover, and rollback); the essentials:

- Image: `rust/Dockerfile` (multi-stage `ubuntu:24.04`) bundles the binary + a
  pinned OR-Tools SDK, so the host only needs Docker. Built + pushed by CI to the
  Codeberg registry, run via `docker-compose.yml` + `deploy/boatsched-docker.service`.
- Data: the SQLite DB is bind-mounted from `/var/lib/boatsched` and reused
  unchanged — the Rust schema/migrations are byte-compatible with the Python DB.
- nginx (`deploy/nginx.conf`) is unchanged: the app still binds `127.0.0.1:8000`
  with the `/collab/` WebSocket upgrade and `X-Real-IP` for login rate-limiting.

```bash
# build + run locally
docker build -t boatsched rust/
docker compose up -d        # needs /etc/boatsched/env and /var/lib/boatsched

# on the box, CI runs the gated cutover for you:
bash /opt/boatsched/deploy/deploy.sh   # backup → validate-on-copy → handover → auto-rollback
```

> The legacy Python bare-metal path (`deploy/boatsched.service` + venv) remains in
> the tree as the rollback target during migration; see git history / older docs
> if you need it.

### Environment

Production reads these environment variables (via the systemd `EnvironmentFile`
or Docker `.env`):

| variable | purpose | required |
|---|---|---|
| `BOATSCHED_ADMIN` | bootstrap admin member name | yes |
| `BOATSCHED_ADMIN_PW` | bootstrap admin password (seeded once) | yes |
| `BOATSCHED_REQUIRE_AUTH` | disable open mode (default on when `--db` set) | no |
| `BOATSCHED_SECURE_COOKIES` | `Secure` flag on session cookie (default on with `--db`) | no |
| `BOATSCHED_MIN_PASSWORD_LEN` | minimum password length (default 8 in prod) | no |
| `BOATSCHED_LOGIN_MAX_FAIL` | failed logins per IP before 429 (default 10) | no |

The database path is still set via the `--db` CLI flag.

---

## Configuration flags (`boatsched-server`)

| flag | meaning | default |
|---|---|---|
| `--db` | SQLite database path | none (in-memory, no persistence) |
| `--port` | listen port | `8000` |
| `--host` | bind address | `0.0.0.0` |
| `--boats` | number of boats | `13` |
| `--levels` | levels low→high | `k b A B` |
| `--adj` | max level-index spread on restricted days | `1` |
| `--restricted` | restricted day names | `Sat Sun Tue Wed` |
| `--diverse` | diverse day names | `Fri Mon` |
| `--stage-seconds` | time budget for solver stages A and B | `45` |
| `--seconds` | time budget for solver stage C | `60` |
| `--workers` | parallel CP-SAT search workers | `8` |

---

## CLI (offline schedule generation)

```bash
# from a roster CSV (columns: name,level)
boatsched --roster examples/roster_52.csv --boats 13 --out schedule.csv

# from per-level counts
boatsched --counts k=12 b=16 A=12 B=12 --boats 13
```

---

## Python library

```python
from boatsched import Config, solve, load_roster

kids = load_roster("examples/roster_52.csv")
rows, stats = solve(kids, Config(boats=13))
# rows: list of (day, boat, name, level)
# stats: {"mixed": int, "distinct": int, "repeats": int, "status_C": str}
```

---

## Solver objective (A > B > C)

| Stage | Objective | Days |
|---|---|---|
| A | Minimise mixed-level boats | `restricted` (Sat/Sun/Tue/Wed by default) |
| B | Maximise distinct levels per boat | `diverse` (Fri/Mon by default) |
| C | Minimise repeat co-boatings (Social-Golfer) | all days |

Stage C usually finishes `FEASIBLE` (good, not proven minimal). Raise `--seconds` to tighten it.

---

## Notes

- In production every route requires authentication; the first admin is seeded from `BOATSCHED_ADMIN` / `BOATSCHED_ADMIN_PW` (see Deployment). In local open mode (no password set) auth is bypassed for convenience.
- Stage C is NP-hard; more `--seconds` gives a better but not necessarily optimal result.
- The Docker image is self-contained — no external services required.
