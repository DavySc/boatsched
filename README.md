# boatsched

A sailing camp management application served as a single, mobile-friendly SPA
(Preact via `preact/compat`, in `web/`) backed by one Rust server, one SQLite database, and one login.
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

See the repository layout in `CLAUDE.md` and the system map in `docs/ARCHITECTURE.md`.

The frontend is a React-API app (aliased to Preact) built with Vite in `web/`, bundled into one self-contained
`index.html` (Yjs is vendored, no CDN) that is compiled into the
`boatsched-server` binary. The server serves it at `/` and exposes the
JSON/WebSocket API it calls. For frontend work run `cd web && npm ci && npm run dev`
(proxies the API to a server on :8000). See `CLAUDE.md` for the full
route and schema reference, and `rust/README.md` for implementation notes.

**New to this codebase or about to make a change?** See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for a full system map plus a
recipe (which files to touch, in what order, which gates you'll trip) for each
kind of change — domain rules, HTTP routes, DB migrations, the solver, the
frontend, auth, and CI/CD.

---

## Local development

### Prerequisites

- Nix with flakes (`nix develop` gives cargo, OR-Tools, Node and sqlite), or
- a Rust toolchain + the OR-Tools SDK for native development (see
  `rust/README.md`).

### Build & run with Nix

```bash
nix build
./result/bin/boatsched-server --db boatsched.db
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

Production runs on a Hetzner box with Nix installed (the box keeps its own
distro). **`deploy/README.md` is the authoritative runbook** (setup, CI secrets,
the gated cutover, and rollback); the essentials:

- Package: `nix build` builds the SPA and the server against nixpkgs' OR-Tools;
  the build runs `cargo test --workspace`, so an untested binary can't exist.
- Service: `deploy/boatsched.service` runs the Nix-built server on
  `127.0.0.1:8000`, behind nginx + certbot (`deploy/nginx.conf`, installed by
  hand) with the `/collab/` WebSocket upgrade and `X-Real-IP` for login
  rate-limiting.
- Data: `/var/lib/boatsched/boatsched.db`, backed up before every deploy to
  `/var/lib/boatsched/backups/`.

```bash
nix build                      # server binary at ./result/bin/boatsched-server
nix develop                    # shell with cargo, OR-Tools, Node, sqlite
deploy/deploy.sh deploy@HOST   # what CI runs on every push to main
```

### Environment

Production reads these environment variables (from `/etc/boatsched/env`, the
service's `EnvironmentFile`):

| variable | purpose | required |
|---|---|---|
| `BOATSCHED_ADMIN` | bootstrap admin member name | yes |
| `BOATSCHED_ADMIN_PW` | bootstrap admin password (seeded once) | yes |
| `BOATSCHED_REQUIRE_AUTH` | disable open mode (default on when `--db` set) | no |
| `BOATSCHED_SECURE_COOKIES` | `Secure` flag on session cookie (default on with `--db`) | no |
| `BOATSCHED_MIN_PASSWORD_LEN` | minimum password length (default 8 in prod) | no |
| `BOATSCHED_LOGIN_MAX_FAIL` | failed logins per IP before 429 (default 10) | no |

The database path is set via the `--db` CLI flag (`deploy/boatsched.service` passes it).

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
</content>
