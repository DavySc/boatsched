# Deployment — boatsched (Rust) on a Hetzner box

CI/CD: on every PR and every push to `main`, a Forgejo Actions **shared runner**
(`codeberg-small`) runs the **`test` job** (`cargo test` over the workspace). On a
push to `main`, the **`deploy` job** then runs (`deploy: needs: test`, so only if
tests pass): it SSHes to the box and runs `deploy/deploy.sh`, which **builds the
image on the box** (`BUILD_ON_HOST=1`) and does the never-break-prod cutover. The
on-box build runs `cargo test` again, so the gate is enforced twice (CI + at
deploy time). The app runs as a **single Docker container** (managed by
`boatsched-docker.service`), bound to `127.0.0.1:8000`, behind nginx + certbot
(TLS terminated by nginx).

> **Why build on the box (no registry):** Codeberg's shared runners can't build
> OCI images (no privileged Docker on shared infra), and Codeberg doesn't accept
> user self-hosted runners. So CI only orchestrates the SSH; the box builds. This
> drops the registry entirely (no `REGISTRY_*` secrets, no `docker login`).

> **The cutover never breaks prod.** Every deploy backs up the DB, validates the
> new image against a *copy* of the production database on a side port, and only
> then does a single-writer handover — with **automatic rollback** if the new
> service doesn't pass its health check. See [Deploy flow](#deploy-flow-automatic).

---

## Why Docker (and why the host OS no longer matters)

The Rust solver links OR-Tools' C++ libraries (`libortools.so` + ~100 bundled
abseil/protobuf/CBC/SCIP/HiGHS libs). Those are ABI-sensitive to glibc/libstdc++.
Containerizing bakes a matching, version-pinned OR-Tools SDK
(**9.15.6755**, `ubuntu-24.04` build) into the image alongside the binary, so the
image is self-contained. **The host only needs Docker** — its distro/version is
irrelevant.

- Build context: `rust/` (`rust/Dockerfile`, multi-stage; builder + runtime both
  `ubuntu:24.04` so the build-time ABI is exactly the run-time ABI).
- The OR-Tools SDK is fetched fresh during the build (gitignored, never vendored
  into git). `crates/solver/build.rs` bakes an rpath to `/opt/or-tools/lib`; the
  runtime stage keeps the libs at that exact path.

---

## Architecture at a glance

```
Internet ──HTTPS──▶ nginx (:443, TLS via certbot)
                      │  proxy_pass + WS upgrade   (deploy/nginx.conf — unchanged)
                      ▼
              boatsched container (127.0.0.1:8000)   ← single container (see below)
                      │  bind mount
              SQLite WAL @ /var/lib/boatsched/boatsched.db   (host path, reused as-is)
```

**Single container — do not scale.** All collab/session/job state lives in the
container's memory (sessions, solve `JOBS`, per-schedule `yrs` docs + the live
WebSockets used to broadcast edits). A second container/replica would not share
them, breaking collaborative editing and sessions. Vertical scaling (bigger box)
is the path, not replicas.

**Data is reused in place.** The container bind-mounts the existing
`/var/lib/boatsched` directory; the Rust DB layer creates tables with
`CREATE TABLE IF NOT EXISTS` and runs the same idempotent migrations as Python,
in WAL mode, with byte-compatible scrypt password hashes. **No data migration,
no schema change, no forced re-login of stored credentials.**

---

## CI secrets (Forgejo repo → Settings → Actions → Secrets)

Only SSH is needed (no registry — the box builds):

| Secret | Contents |
|---|---|
| `SSH_PRIVATE_KEY` | Private half of the deploy keypair (full PEM). |
| `SSH_HOST` | Box hostname or public IP. |
| `SSH_USER` | `deploy` |
| `SSH_KNOWN_HOSTS` | Pinned host key line(s) from `ssh-keyscan`. |

---

## CI runner notes

`.forgejo/workflows/deploy.yml` has two jobs, both on Codeberg's shared runner
(`runs-on: codeberg-small`):

- **`test`** — runs on every PR and every push to `main`. In an `ubuntu:24.04`
  container (matching the OR-Tools ubuntu-24.04 SDK ABI, like `rust/Dockerfile`),
  it installs the pinned Rust toolchain, fetches the OR-Tools SDK, and runs
  `cargo test --workspace --locked`. Every step is shell-only (no JS actions), so
  the container needs no Node — it clones the repo manually.
- **`deploy`** — runs only on a push to `main` and only if `test` passed
  (`needs: test`). It SSHes to the box and runs `deploy/deploy.sh`; it does **not**
  build or push an image. The box builds it (`BUILD_ON_HOST=1` in
  `/etc/boatsched/env`), and that on-box build runs `cargo test` again — so the
  gate holds even if you deploy by hand.

Why the box builds (not CI):

- Codeberg's **shared runners can't build OCI images** (no privileged Docker), and
- Codeberg does **not** accept user **self-hosted** runners (registration is
  refused — `runner registration token not found`), so a build-on-CI job has
  nowhere to run.

The `test` job still runs `cargo test` directly (no Docker needed) — only *image
building* is the problem on shared runners, not compiling/testing. Building on the
box compiles during the deploy (CPU/RAM contention for a few minutes) but does
**not** disturb the running service — the new image is only swapped in after it
validates on `:8001`.

> **Want build-on-CI later?** If you move to a self-hosted Forgejo instance with a
> runner that can build images, restore the registry `build` job (in git history)
> and unset `BUILD_ON_HOST`. Not possible on `codeberg.org`.

> **Fully manual** (no CI at all) also works: `sudo -u deploy bash
> /opt/boatsched/deploy/deploy.sh` on the box.

---

## One-time server provisioning (clean box)

This is the **clean-box** path — provisioning a fresh host for the container. If a
Python boatsched is **already live with data**, do **not** follow this top-to-bottom;
use [deploy/CUTOVER_CHECKLIST.md](./CUTOVER_CHECKLIST.md), which reuses the existing
user/DB/env and only adds the Docker delta.

Run as root (or via `sudo`).

### 1. Install Docker + helpers

```bash
apt update
apt install -y docker.io git nginx sqlite3 curl
systemctl enable --now docker
```

The scripts use Compose **v2** (`docker compose`, with a space). The
`docker-compose-plugin` package only exists in Docker's own apt repo, **not** with
the distro's `docker.io`. Install the v2 plugin one of these ways:

```bash
# A) distro package (Ubuntu 24.04 / Debian 13+):
apt install -y docker-compose-v2

# B) distro-agnostic — drop the plugin binary in (works alongside docker.io):
mkdir -p /usr/local/lib/docker/cli-plugins
curl -SL "https://github.com/docker/compose/releases/latest/download/docker-compose-linux-$(uname -m)" \
  -o /usr/local/lib/docker/cli-plugins/docker-compose
chmod +x /usr/local/lib/docker/cli-plugins/docker-compose

docker compose version   # must report v2.x
```

### 2. Create the `deploy` user (in the `docker` group)

```bash
useradd --create-home --shell /bin/bash deploy
usermod -aG docker deploy
```

### 3. Clone the repo to the deploy path

```bash
install -d -o deploy -g deploy /opt/boatsched
sudo -u deploy git clone https://codeberg.org/Oventosti/boatsched.git /opt/boatsched
```

### 4. (Registry login — not needed)

The box **builds** the image (`BUILD_ON_HOST=1`, see step 6), so there is no
registry pull and no `docker login`. Skip this. (Only needed if you switch to a
registry-based build on a self-hosted CI runner.)

### 5. DB directory (local NVMe, owned by deploy)

```bash
install -d -o deploy -g deploy /var/lib/boatsched
install -d -o deploy -g deploy /var/lib/boatsched-validate   # validation copy
```

If migrating from the Python deployment, the existing
`/var/lib/boatsched/boatsched.db` (+ `-wal`/`-shm`) is reused unchanged.

### 6. Env file `/etc/boatsched/env`

Same file as before — admin bootstrap + solver workers. `docker-compose.yml`
passes it via `env_file`.

```bash
install -d -o root -g root -m 755 /etc/boatsched
cat > /etc/boatsched/env <<'EOF'
# Bootstrap admin — seeded on first start (idempotent; UI password changes persist).
BOATSCHED_ADMIN=yourname
BOATSCHED_ADMIN_PW=use-a-long-random-password

# CP-SAT search workers (forwarded into --workers by the container entrypoint).
SOLVER_WORKERS=2

# Build the image ON THE BOX during deploy (Codeberg CI can't build images).
BUILD_ON_HOST=1

# require_auth + secure_cookies auto-enable because the server runs with --db.
EOF
chown root:deploy /etc/boatsched/env
chmod 640 /etc/boatsched/env
```

### 7. Install the systemd unit

```bash
cp /opt/boatsched/deploy/boatsched-docker.service /etc/systemd/system/
systemctl daemon-reload
# Do NOT enable yet if a Python boatsched.service is currently serving — the
# first cutover (deploy.sh) performs the handover. For a fresh box:
systemctl enable --now boatsched-docker
systemctl status boatsched-docker
```

### 8. Scoped sudoers for the deploy user

`deploy.sh` starts/stops both services (handover + rollback). Grant exactly those.

```bash
cat > /etc/sudoers.d/boatsched-deploy <<'EOF'
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl start boatsched, \
                            /usr/bin/systemctl stop boatsched, \
                            /usr/bin/systemctl restart boatsched, \
                            /usr/bin/systemctl start boatsched-docker, \
                            /usr/bin/systemctl stop boatsched-docker, \
                            /usr/bin/systemctl restart boatsched-docker
EOF
chmod 440 /etc/sudoers.d/boatsched-deploy
visudo -cf /etc/sudoers.d/boatsched-deploy
```

> Confirm the `systemctl` path (`command -v systemctl`). The paths in the sudoers
> line must be exact.

### 9. Deploy key + host key

Unchanged from before: authorize the CI deploy public key in the `deploy` user's
`~/.ssh/authorized_keys`, and pin the box's host key into `SSH_KNOWN_HOSTS` via
`ssh-keyscan`.

---

## nginx + TLS — unchanged

`deploy/nginx.conf` is **untouched** by this migration: the app still listens on
`127.0.0.1:8000`, and the `/collab/` WebSocket upgrade + the `X-Real-IP` header
the app reads for login rate-limiting are the same. If nginx is already
configured for the Python deployment, **no nginx change is required.**

---

## Migrating from a live Python deployment (first cutover)

The provisioning above is the **clean-box** path. If a Python boatsched is
**already running with live data**, follow the dedicated step-by-step runbook
instead — it covers only the delta (the box is already provisioned for Python) and
the safe, never-break-prod handover:

➡️ **[deploy/CUTOVER_CHECKLIST.md](./CUTOVER_CHECKLIST.md)**

---

## Deploy flow (automatic)

1. Push to `main` on Codeberg.
2. **test** job (shared runner `codeberg-small`): `cargo test --workspace`. The
   deploy is gated on this (`deploy: needs: test`) — a red test means no deploy.
3. **deploy** job (shared runner `codeberg-small`): SSHes as `deploy` and runs
   `deploy/deploy.sh`, which:
   - `git pull --ff-only` (compose/unit/script updates),
   - backs up the live DB (`sqlite3 .backup`),
   - `docker compose build` on the box (`BUILD_ON_HOST=1`) — compiles + runs the
     `cargo test` gate; a failing test fails the deploy here,
   - **validates** the image on `:8001` against a DB **copy** (health, `GET /`,
     and an admin login that exercises DB read + scrypt) — aborts here, prod
     untouched, if anything fails,
   - **handover**: `systemctl stop boatsched` (Python releases the DB) →
     `systemctl restart boatsched-docker` (container opens the live DB on `:8000`),
   - **health-gates** `:8000`; on failure **auto-rolls back** to the Python
     service or the previous image,
   - prunes dangling images and prints the deployed image + DB backup path.

There is **no separate migration step** — the Rust server runs idempotent schema
migrations at startup, so a (re)start applies any schema changes.

---

## Manual rollback

```bash
# Back to Python (if still installed):
sudo systemctl stop boatsched-docker
sudo systemctl start boatsched

# Or to the previous image:
docker tag <previous-image-id> codeberg.org/oventosti/boatsched:latest
sudo systemctl restart boatsched-docker
```

The most recent `pre-cutover-*.db` backup in `/var/lib/boatsched` is a restore
point for the data itself.

---

## Capacity, solver, single-process assumptions

Unchanged in substance from the Python deployment: one process/container, in-memory
sessions/jobs/collab (a restart drops them), `SOLVER_WORKERS=2` to leave headroom
on the shared vCPUs. OR-Tools maps executable memory, so do **not** add a
seccomp/no-exec profile that blocks it.
