# boatsched

A sailing camp management application served as a single, mobile-friendly SPA
(`viewer.html`) backed by one Rust server, one SQLite database, and one login.
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
├── rust/                  Rust workspace — the production server
│   ├── crates/core        domain types + invariants
│   ├── crates/roster      roster CSV ingestion
│   ├── crates/db          SQLite access (rusqlite) + migrations
│   ├── crates/solver      OR-Tools CP-SAT model (C++ FFI)
│   ├── crates/auth        sessions, scrypt passwords, rate-limiting
│   ├── crates/server      Axum HTTP + WebSocket app (binary: boatsched-server)
│   └── Dockerfile         multi-stage image (binary + pinned OR-Tools SDK)
├── src/boatsched/
│   └── viewer.html        the entire SPA (vanilla JS) — embedded into the
│                          server binary at compile time via include_str!
├── spec/                  Lean 4 formal model of record (backs the Rust impl)
├── examples/roster_52.csv sample roster (used by the solver benchmark)
├── deploy/                nginx + systemd config for the Hetzner box
└── docker-compose.yml     production stack (Rust container)
```

There is **no separate frontend build** — `viewer.html` is a self-contained
single-page app (it loads Yjs from esm.sh for collaborative editing) and is
compiled directly into the `boatsched-server` binary. The server serves it at
`/` and exposes the JSON/WebSocket API it calls. See `CLAUDE.md` for the full
route and schema reference, and `rust/README.md` for implementation notes.

---

## Local development

### Prerequisites

- Docker (the only requirement for a run-it-locally build), or
- a Rust toolchain + the OR-Tools SDK for native development (see
  `rust/README.md`).

### Build & run with Docker

The image is self-contained (binary + OR-Tools native libs). The build context
is the **repo root** because the binary embeds `src/boatsched/viewer.html`:

```bash
docker build -t boatsched -f rust/Dockerfile .
docker compose up -d      # needs /etc/boatsched/env and /var/lib/boatsched
# App at http://localhost:8000
```

### Build & run natively

```bash
cd rust
cargo run -p boatsched-server -- --db boatsched.db
# App at http://localhost:8000
```

With a fresh local database and no admin/password set, the server runs in **open
mode** — any login is accepted so you can start immediately. Set a password on an
instructor (or run with the production env vars below) to switch on real auth.

### Tests

```bash
cd rust
cargo test --workspace            # the full Rust test suite (the CI gate)
```

The Lean spec is verified separately with `cd spec && lake build`.

---

## Deployment on Hetzner

> **Security model (read first).** In production the app refuses anonymous
> access: `--db` turns on `require_auth`, `secure_cookies`, an 8-char minimum
> password, and login rate-limiting. There is **no open/bootstrap window** —
> the first admin is seeded from environment variables, so you must set
> `BOATSCHED_ADMIN` and `BOATSCHED_ADMIN_PW` before first start or nobody can
> log in. Always run behind the provided nginx + TLS config; never expose port
> 8000 to the internet directly.

Production runs the Rust server as a single Docker container behind
nginx+certbot. **`deploy/README.md` is the authoritative runbook** (provisioning,
CI secrets, the never-break-prod cutover, and rollback); the essentials:

- Image: `rust/Dockerfile` (multi-stage `ubuntu:24.04`) bundles the binary + a
  pinned OR-Tools SDK, so the host only needs Docker. Built on the box by CI and
  run via `docker-compose.yml` + `deploy/boatsched-docker.service`.
- Data: the SQLite DB is bind-mounted from `/var/lib/boatsched` and reused
  across deploys; the Rust server runs idempotent migrations at startup.
- nginx (`deploy/nginx.conf`): the app binds `127.0.0.1:8000` with the
  `/collab/` WebSocket upgrade and `X-Real-IP` for login rate-limiting.

```bash
# build + run locally
docker build -t boatsched -f rust/Dockerfile .
docker compose up -d        # needs /etc/boatsched/env and /var/lib/boatsched

# on the box, CI runs the gated cutover for you:
bash /opt/boatsched/deploy/deploy.sh   # backup → validate-on-copy → handover → auto-rollback
```

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

The database path is set via the `--db` CLI flag (the container entrypoint
passes `BOATSCHED_DB`).

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
</content>
