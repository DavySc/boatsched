# Deployment — boatsched on a Hetzner cx23

CI/CD: push to `main` on Codeberg (`Oventosti/boatsched`) → Forgejo Actions runs
the test job, then SSHes to the box and runs `deploy/deploy.sh` (git pull +
venv sync + `systemctl restart`). The app runs as a systemd service in a venv,
bound to `127.0.0.1:8000`, behind nginx + certbot (TLS terminated by nginx).

> **Heads-up — this started life as a local single-user tool.** Before exposing
> it publicly, review the [single-user assumptions](#single-user-assumptions-review-before-public)
> at the bottom. The production hardening (auth, TLS, rate limiting) is in place,
> but the collab/job state is deliberately in-process (see worker note below).

---

## Architecture at a glance

```
Internet ──HTTPS──▶ nginx (:443, TLS via certbot)
                      │  proxy_pass + WS upgrade
                      ▼
                 uvicorn (127.0.0.1:8000)   ← single process (see below)
                      │
              SQLite WAL @ /var/lib/boatsched/boatsched.db
```

**Single web process — do not scale workers.** All collab/session/job state
lives in this process's memory (`docs`, `rooms`, `sessions`, `JOBS` in
`server.py`); `rooms` even holds live WebSocket objects used to broadcast Yjs
updates. A second worker would not share them, breaking collaborative editing
and sessions. The service runs the `boatsched-server` console script, which is a
single `uvicorn.run(...)` process. Vertical scaling (bigger box) is the path, not
more web workers.

---

## CI secrets (set in the Forgejo repo → Settings → Actions → Secrets)

| Secret | Contents |
|---|---|
| `SSH_PRIVATE_KEY` | The **private** half of the deploy keypair (full PEM, including the `-----BEGIN/END-----` lines). Matches the public key in the `deploy` user's `~/.ssh/authorized_keys`. |
| `SSH_HOST` | The box's hostname or public IP (e.g. `boatsched.example.com`). |
| `SSH_USER` | `deploy` |
| `SSH_KNOWN_HOSTS` | The pinned host key line(s) from `ssh-keyscan` (see below). Pins the server's identity so the workflow never uses `StrictHostKeyChecking=no`. |

---

## CI runner notes (Forgejo-instance-specific)

`.forgejo/workflows/deploy.yml` assumes:

- A runner registered with the **`docker`** label (`runs-on: docker`). If your
  Codeberg/Forgejo runner uses a different label, change `runs-on` to match.
- The runner can resolve `actions/checkout@v4`. Forgejo proxies the actions
  ecosystem (commonly from `https://github.com` or `https://code.forgejo.org`);
  if checkout fails to fetch, point it at the source your instance allows (e.g.
  `https://code.forgejo.org/actions/checkout`) or set the runner's default
  actions URL.

---

## One-time server provisioning

Run as root (or via `sudo`) on a fresh cx23.

### 1. Base packages

```bash
apt update
apt install -y python3.11 python3.11-venv python3-pip git nginx
```

### 2. Create the `deploy` user

```bash
useradd --create-home --shell /bin/bash deploy
```

### 3. Clone the repo to the deploy path

```bash
install -d -o deploy -g deploy /opt/boatsched
sudo -u deploy git clone https://codeberg.org/Oventosti/boatsched.git /opt/boatsched
```

### 4. Create the venv and install the package

```bash
sudo -u deploy python3.11 -m venv /opt/boatsched/.venv
sudo -u deploy /opt/boatsched/.venv/bin/pip install --upgrade pip
sudo -u deploy /opt/boatsched/.venv/bin/pip install -e /opt/boatsched
```

### 5. Create the DB directory (local NVMe, owned by deploy)

```bash
install -d -o deploy -g deploy /var/lib/boatsched
```

The WAL/SHM sidecar files (`boatsched.db-wal`, `-shm`) will live here on local
disk. These are gitignored and must never be committed.

### 6. Create the env file `/etc/boatsched/env`

Holds the admin bootstrap credentials **and** the solver worker count. Not in
git. Keep it readable only by root and the `deploy` group.

```bash
install -d -o root -g root -m 755 /etc/boatsched
cat > /etc/boatsched/env <<'EOF'
# Bootstrap admin — seeded on first start (idempotent; UI password changes persist).
BOATSCHED_ADMIN=yourname
BOATSCHED_ADMIN_PW=use-a-long-random-password

# CP-SAT search workers. The box has 2 shared vCPUs; keep headroom (upstream
# default is 8, which would peg both cores for the whole solve). Solves are rare
# and off-peak, so a slower solve is fine.
SOLVER_WORKERS=2

# require_auth + secure_cookies auto-enable because the service runs with --db.
# Override only if you know why:
# BOATSCHED_REQUIRE_AUTH=1
# BOATSCHED_SECURE_COOKIES=1
# BOATSCHED_MIN_PASSWORD_LEN=8
EOF
chown root:deploy /etc/boatsched/env
chmod 640 /etc/boatsched/env
```

### 7. Install the systemd unit

```bash
cp /opt/boatsched/deploy/boatsched.service /etc/systemd/system/boatsched.service
systemctl daemon-reload
systemctl enable --now boatsched
systemctl status boatsched      # confirm it's active
```

### 8. Scoped sudoers for the deploy user

`deploy/deploy.sh` runs `sudo systemctl restart boatsched`. Grant exactly that —
nothing broader. Use `visudo` to validate.

```bash
cat > /etc/sudoers.d/boatsched-deploy <<'EOF'
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl restart boatsched
EOF
chmod 440 /etc/sudoers.d/boatsched-deploy
visudo -cf /etc/sudoers.d/boatsched-deploy
```

> Confirm the `systemctl` path matches the box (`command -v systemctl` — usually
> `/usr/bin/systemctl` on Debian/Ubuntu). The path in the sudoers line must be
> exact for the rule to match.

### 9. Authorize the CI deploy key

Generate the keypair (on your workstation, **not** on the box):

```bash
ssh-keygen -t ed25519 -f ./boatsched_deploy -C "forgejo-ci-deploy" -N ""
# boatsched_deploy      → paste into the SSH_PRIVATE_KEY secret
# boatsched_deploy.pub  → install on the box (below)
```

Install the public key for the `deploy` user:

```bash
install -d -o deploy -g deploy -m 700 /home/deploy/.ssh
# paste boatsched_deploy.pub as a line in:
sudo -u deploy tee -a /home/deploy/.ssh/authorized_keys < boatsched_deploy.pub
chmod 600 /home/deploy/.ssh/authorized_keys
```

### 10. Pin the host key for `SSH_KNOWN_HOSTS`

```bash
ssh-keyscan -t ed25519 boatsched.example.com   # or the IP
# paste the resulting line(s) into the SSH_KNOWN_HOSTS secret
```

---

## nginx + TLS

A complete server block lives in [`deploy/nginx.conf`](./nginx.conf) (TLS,
security headers, rate-limit zones, and the WebSocket location). Install it as an
`http`-context file and let certbot manage the cert:

```bash
cp /opt/boatsched/deploy/nginx.conf /etc/nginx/conf.d/boatsched.conf
# edit /etc/nginx/conf.d/boatsched.conf → set server_name to your domain
apt install -y certbot python3-certbot-nginx
certbot --nginx -d boatsched.example.com    # provisions + auto-renews TLS
nginx -t && systemctl reload nginx
```

### The WebSocket gotcha (the #1 thing people break)

Yjs collaborative editing is a WebSocket at `/collab/{schedule_id}`. nginx will
silently drop the `Upgrade`/`Connection` handshake on a plain `proxy_pass` unless
you set the upgrade headers. The minimum that must be present:

```nginx
location /collab/ {
    proxy_pass http://127.0.0.1:8000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;          # <-- required for WS
    proxy_set_header Connection "upgrade";           # <-- required for WS
    proxy_set_header Host $host;
    proxy_read_timeout 3600s;                        # keep long-lived sockets open
    proxy_send_timeout 3600s;
}

location / {
    proxy_pass http://127.0.0.1:8000;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;         # app reads this for login rate-limiting
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

The frontend already chooses `wss://` when the page is served over HTTPS, so once
TLS is on, collab works end-to-end. `deploy/nginx.conf` already includes all of
the above.

---

## Deploy flow (automatic)

1. Push to `main` on Codeberg.
2. Forgejo Actions **test** job: `pip install ".[dev]"` → `pytest`. Deploy is
   gated on this passing.
3. **deploy** job: SSHes as `deploy` (host key pinned via `SSH_KNOWN_HOSTS`) and
   runs `/opt/boatsched/deploy/deploy.sh`, which:
   - `git pull --ff-only origin main` (diverged prod hard-fails — no merge on prod),
   - `pip install -e .` into the venv,
   - `sudo systemctl restart boatsched` (migrations run at startup),
   - prints the deployed commit hash.

There is **no separate migration step** — `create_app()` runs idempotent schema
migrations on startup (`server.py:240-247`), so a restart is all that's needed.

---

## Manual rollback

SSH to the box as `deploy`, check out the previous commit, and restart:

```bash
cd /opt/boatsched
git log --oneline -n 5                  # find the last-good commit
git checkout <prev-commit-sha>          # detached HEAD on the known-good build
sudo systemctl restart boatsched
systemctl is-active boatsched && git rev-parse HEAD
```

To return to tracking `main` later: `git checkout main`. (The next CI deploy runs
`git pull --ff-only`; if you left the checkout on a detached HEAD or an older
commit, fast-forward will still advance cleanly to `origin/main`.)

---

## Capacity & solver

The cx23 (2 shared vCPUs, 4 GB RAM, 40 GB NVMe) is sized for the expected load:
**~25 concurrent users doing progress-tracking data entry**, which is light —
mostly small JSON writes and one long-lived WebSocket per editor.

The **solver is the only heavy workload.** A staged CP-SAT solve runs up to
~`stage_seconds*2 + seconds` (~150 s at defaults). Important specifics:

- The solve runs in a **background daemon thread** (`server.py:624`), not on the
  event loop, and OR-Tools' `Solve()` releases the GIL — so a solve does **not**
  freeze HTTP/WebSocket traffic. The real effect is **CPU contention**: CP-SAT's
  search workers consume vCPUs, so collab can feel laggy *during* a solve.
- It's pinned to `SOLVER_WORKERS=2` (via `/etc/boatsched/env` → `--workers`) to
  leave headroom on the 2 shared vCPUs.
- Solves are expected to be run **off-peak**, when users aren't editing. An admin
  *could* trigger a solve mid-session; if that ever becomes routine, the path is
  to move to a **cx33** (more vCPUs) and/or offload the solve to a separate
  process/queue so it can't contend with interactive traffic at all. Today it's a
  deliberate, acceptable tradeoff, not a bug.

---

## Single-user assumptions (review before public)

This project began as a local single-user tool. Things to keep in mind now that
it's internet-facing — **noted for review, not fixed here:**

- **In-process job & collab state** (`JOBS`, `sessions`, `docs`, `rooms` in
  `server.py`). This is what forces a single web process. It also means a
  restart drops in-flight solve jobs (`/status` 404s after restart) and active
  sessions (users re-login). Fine for this scale; revisit if you ever need HA or
  multiple workers (would require Redis/external session + a shared CRDT relay).
- **Solve concurrency** — one global solve at a time is fine for off-peak use,
  but there's no queue/limit on concurrent solves beyond their CPU cost.
- **Auth coverage** — `/`, `/index.html`, `/api/auth/login`, `/api/auth/logout`,
  and `/api/health` are intentionally public; everything else requires the
  `bsession` cookie. Worth re-confirming no new endpoint slips out of that net
  when routes are added.
- **Eval-card persistence gap** — `viewer.html` calls `/api/progress`, which has
  no backend route yet (calls fail silently). Not a security issue, but note that
  skill evaluations don't persist until that's wired up.
