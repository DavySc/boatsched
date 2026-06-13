# boatsched

A sailing camp management application with two integrated tools:

- **Zeilkamp App** — mobile-friendly evaluation tracker for instructors: manage campers, fill in daily skill evaluation cards, and record manual boat allocations.
- **Rooster Optimizer** — schedule generator that assigns kids to boats over multiple days under level constraints, optimising a strict lexicographic objective with OR-Tools CP-SAT.

Both tools share one Python backend, one SQLite database, and one login.

---

## Architecture overview

```
boatsched/
├── frontend/          React + Vite + Tailwind v4 (Zeilkamp App UI)
│   ├── src/
│   └── dist/          built output (gitignored; served by Python server)
├── src/boatsched/     Python package (FastAPI server + solver + DB)
│   ├── server.py
│   ├── solver.py
│   ├── db.py
│   ├── roster.py
│   ├── cli.py
│   └── viewer.html    Rooster Optimizer SPA (embedded)
├── deploy/            nginx + systemd config for bare-metal Hetzner
├── Dockerfile
└── docker-compose.yml
```

The Python server at runtime:
- Serves the React SPA at `/` (from `frontend/dist/` when built)
- Serves the Rooster Optimizer at `/optimizer`
- Exposes a REST API at `/api/` for the React app
- Exposes the existing schedule/solver API for the optimizer

---

## Local development

### Prerequisites

- Python 3.11+
- Node.js 20+

### Python backend

```bash
python -m venv .venv
source .venv/bin/activate          # Windows: .venv\Scripts\activate
pip install -e ".[dev]"
```

### Frontend

```bash
cd frontend
npm install
```

### Running locally

Start the Python backend (in one terminal):

```bash
source .venv/bin/activate
boatsched-server --db boatsched.db
# Listening on http://localhost:8000
```

Start the Vite dev server (in another terminal) — it proxies `/api` and `/optimizer` to the Python server:

```bash
cd frontend
npm run dev
# Zeilkamp App at http://localhost:5173
# Optimizer at  http://localhost:5173/optimizer
```

On first startup with a fresh database the server redirects to `/setup` where you create the first instructor account.

### Tests

```bash
pytest                                          # all tests with coverage
pytest tests/test_boatsched.py::test_tiny_solve # single test
```

---

## Deployment on Hetzner

### Option A — Docker (recommended)

Build and start everything with Docker Compose. The build step compiles the React app and packages it with the Python server into a single image.

```bash
# on your local machine
docker compose build
docker compose push   # if using a registry, otherwise build on the server

# on the Hetzner server
git clone <repo> /opt/boatsched
cd /opt/boatsched
docker compose up -d
```

The service runs on port 8000 inside Docker; add a reverse proxy in front if you need HTTPS.

`docker-compose.yml` mounts a named volume at `/data` for the SQLite database so it persists across container rebuilds.

### Option B — nginx + systemd (bare-metal)

**1. Install Python and OR-Tools dependencies:**

```bash
apt install python3.11 python3.11-venv python3-pip nginx
```

**2. Build the frontend:**

```bash
cd /opt/boatsched/frontend
npm ci
npm run build
```

**3. Install the Python package:**

```bash
cd /opt/boatsched
python3.11 -m venv .venv
.venv/bin/pip install -e .
```

**4. Create a system user and data directory:**

```bash
useradd --system --home /opt/boatsched boatsched
mkdir -p /var/lib/boatsched
chown boatsched /var/lib/boatsched
```

**5. Install and enable the systemd service:**

```bash
cp deploy/boatsched.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now boatsched
```

**6. Configure nginx:**

```bash
cp deploy/nginx.conf /etc/nginx/sites-available/boatsched
ln -s /etc/nginx/sites-available/boatsched /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
```

For HTTPS, use Certbot:

```bash
apt install certbot python3-certbot-nginx
certbot --nginx -d yourdomain.example
```

### Environment

No environment variables are required. All configuration is passed as CLI flags to `boatsched-server`. The database path defaults to `/data/boatsched.db` in Docker and `/var/lib/boatsched/boatsched.db` in the systemd setup.

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

- The server enforces authentication for all routes. On first run it redirects to `/setup` (only accessible before any instructor account exists) to create the first account.
- Stage C is NP-hard; more `--seconds` gives a better but not necessarily optimal result.
- The Docker image is self-contained — no external services required.
