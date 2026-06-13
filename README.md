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

### Option A — Docker (recommended)

Build and start everything with Docker Compose. The image binds the app to
`127.0.0.1:8000` only — put nginx (TLS, rate limiting, headers) in front.

```bash
# on the Hetzner server
git clone <repo> /opt/boatsched
cd /opt/boatsched

# Create the secrets file (NOT committed). chmod 600.
cat > .env <<'EOF'
BOATSCHED_ADMIN=yourname
BOATSCHED_ADMIN_PW=use-a-long-random-password
EOF
chmod 600 .env

docker compose up -d --build
```

`docker-compose.yml` sets `BOATSCHED_REQUIRE_AUTH=1`, `BOATSCHED_SECURE_COOKIES=1`,
and a minimum password length, and mounts a named volume at `/data` for the
SQLite database. Then configure nginx as in Option B step 6 below so HTTPS
terminates in front of the container.

### Option B — nginx + systemd (bare-metal)

**1. Install Python and OR-Tools dependencies:**

```bash
apt install python3.11 python3.11-venv python3-pip nginx
```

**2. Install the Python package:**

```bash
cd /opt/boatsched
python3.11 -m venv .venv
.venv/bin/pip install -e .
```

**3. Create a system user and data directory:**

```bash
useradd --system --home /opt/boatsched boatsched
mkdir -p /var/lib/boatsched
chown boatsched /var/lib/boatsched
```

**4. Create the secrets / env file (root-owned, 0600):**

The systemd unit loads `/etc/boatsched/boatsched.env`. It **must** contain the
bootstrap admin credentials, or the service starts locked (nobody can log in):

```bash
mkdir -p /etc/boatsched
cat > /etc/boatsched/boatsched.env <<'EOF'
BOATSCHED_ADMIN=yourname
BOATSCHED_ADMIN_PW=use-a-long-random-password
EOF
chmod 600 /etc/boatsched/boatsched.env
chown root:root /etc/boatsched/boatsched.env
```

Production hardening (`require_auth`, secure cookies, 8-char min password) is on
automatically because the service runs with `--db`. The admin password is seeded
on first start; changing it later in the UI persists across restarts.

**5. Install and enable the systemd service:**

```bash
cp deploy/boatsched.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now boatsched
```

**6. Configure nginx (TLS, rate limiting, security headers):**

The provided config is an `http`-context file (it defines `limit_req_zone`),
so install it under `conf.d/`, not `sites-available/`. Edit `server_name` to
your domain first.

```bash
cp deploy/nginx.conf /etc/nginx/conf.d/boatsched.conf
# edit /etc/nginx/conf.d/boatsched.conf → set server_name to your domain
apt install certbot python3-certbot-nginx
certbot --nginx -d yourdomain.example      # provisions/renews the TLS cert
nginx -t && systemctl reload nginx
```

Confirm the app port is **not** publicly reachable: `curl http://YOUR_IP:8000`
from another host should fail (the app binds `127.0.0.1` only).

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
