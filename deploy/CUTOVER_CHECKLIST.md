# Manual cutover checklist — boatsched Python → Rust container

Everything you must do **by hand** for the swapover to succeed, in order. The
automated parts (CI build/push, `deploy/deploy.sh` cutover with auto-rollback)
are covered in `deploy/README.md`; this file is only the manual prerequisites and
the one blocker found while testing the deploy strategy locally.

---

## 0. Build blockers — FIXED in code (verified by a clean build + run)

Two build blockers were found and fixed (commit these). Both were because the
image was built from `rust/`, but the server crate embeds the shared SPA from the
**repo root** (`include_str!("../../../../src/boatsched/viewer.html")`), which is
outside that context. The fix builds from the repo root instead:

1. `rust/Dockerfile` — build context is now the repo root; the layout is mirrored
   in the image (workspace at `/build/rust`, viewer at
   `/build/src/boatsched/viewer.html`), built from `/build/rust`. Also fixed
   `COPY docker-entrypoint.sh` → `COPY rust/docker-entrypoint.sh` (same cause).
2. `docker-compose.yml` + `docker-compose.validate.yml` —
   `build: { context: ., dockerfile: rust/Dockerfile }`.
3. `.forgejo/workflows/deploy.yml` — `docker build … -f rust/Dockerfile .`.
4. `rust/.dockerignore` moved to repo-root `.dockerignore` (context changed),
   excluding `rust/target/`, `rust/vendor/` (the wrong-ABI Arch SDK), `.git/`,
   `.venv/`, `*.db*`, etc.

Verified locally: `docker build -f rust/Dockerfile .` builds clean, the in-image
`cargo test` gate passes (all suites incl. the new eval tests), and the running
container answers `/api/health` 200, serves the SPA, admin login 200 / bad-pw 401,
and `/api/skills` returns the DB-seeded catalog. **No action needed beyond
committing these files** — included here only so you know what changed.

---

## 0b. RUNTIME requirement — data dir must be writable by uid 10001 ⚠️

The image runs as the non-root user `boatsched` (**uid 10001**). The bind-mounted
data dir must be owned/writable by that uid, or the container **crashes on boot**:

```
[error] could not open database /data/boatsched.db: unable to open database file
```

The README's `install -d -o deploy -g deploy /var/lib/boatsched` makes the dir
owned by `deploy` (a different uid) → the container can't write. **On the box:**

```bash
sudo chown -R 10001:10001 /var/lib/boatsched /var/lib/boatsched-validate
```

When migrating from Python, also chown the **existing** DB files (contents are
reused unchanged, but ownership must move to 10001):

```bash
sudo chown 10001:10001 /var/lib/boatsched/boatsched.db*
```

(Alternative: add `user: "<deploy-uid>:<deploy-gid>"` to both compose files so the
container runs as the dir's owner — but the deterministic `chown` to 10001 matches
the image's fixed uid and is simpler.)

---

## 1. Get the code onto `main`

The deploy triggers on **push to `main`** and `deploy.sh` does `git pull --ff-only
origin main`. The work is currently on `rust-rewrite`.

- [ ] Apply the blocker fix above.
- [ ] Merge `rust-rewrite` → `main` (or whatever branch CI watches).
- [ ] Confirm `main` builds the image end-to-end (CI green, or a manual `docker build`).

---

## 2. Forgejo CI secrets (repo → Settings → Actions → Secrets)

- [ ] `REGISTRY_USER` — Codeberg user/bot with package write
- [ ] `REGISTRY_TOKEN` — Codeberg token, `write:package` scope
- [ ] `SSH_PRIVATE_KEY` — full PEM of the deploy keypair
- [ ] `SSH_HOST` — box hostname / IP
- [ ] `SSH_USER` — `deploy`
- [ ] `SSH_KNOWN_HOSTS` — pinned host key from `ssh-keyscan <host>`
- [ ] Confirm the registry path/casing in the workflow + compose
      (`codeberg.org/oventosti/boatsched`) matches the actual Codeberg package.
- [ ] If your runner can't build OCI images, set `BUILD_ON_HOST=1` (in
      `/etc/boatsched/env`) and drop the `build` job — see README.

---

## 3. One-time server provisioning (run on the box as root)

- [ ] `apt install -y docker.io docker-compose-plugin git nginx sqlite3 curl`;
      `systemctl enable --now docker`
- [ ] `useradd --create-home --shell /bin/bash deploy` ; `usermod -aG docker deploy`
- [ ] `git clone …/boatsched.git /opt/boatsched` (owned by `deploy`)
- [ ] `sudo -u deploy docker login codeberg.org` (registry pull; needs `read:package`)
- [ ] `install -d /var/lib/boatsched /var/lib/boatsched-validate` then
      `chown -R 10001:10001 /var/lib/boatsched /var/lib/boatsched-validate`
      (the container runs as uid 10001 — see §0b; do NOT leave it owned by `deploy`)
- [ ] Create `/etc/boatsched/env` (root:deploy, **chmod 640**):
      `BOATSCHED_ADMIN`, `BOATSCHED_ADMIN_PW` (≥8 chars), `SOLVER_WORKERS=2`
- [ ] Install the unit: `cp deploy/boatsched-docker.service /etc/systemd/system/` ;
      `systemctl daemon-reload`
      - Fresh box: `systemctl enable --now boatsched-docker`
      - **Cutover from Python: do NOT enable it yet** — the first `deploy.sh` run
        does the handover.
- [ ] Scoped sudoers `/etc/sudoers.d/boatsched-deploy` (chmod 440, `visudo -cf`)
      allowing `deploy` to `systemctl start|stop|restart` **both** `boatsched` and
      `boatsched-docker`. Verify the `systemctl` path with `command -v systemctl`.
- [ ] Authorize the CI deploy **public** key in `~deploy/.ssh/authorized_keys`.

---

## 4. nginx + TLS (manual, on the box)

- [ ] Install `deploy/nginx.conf` → `/etc/nginx/conf.d/boatsched.conf`
- [ ] Replace `example.com` with the real domain in **both** server blocks +
      the `ssl_certificate*` paths
- [ ] `certbot --nginx -d <domain>` (or place certs manually)
- [ ] `nginx -t && systemctl reload nginx`
- [ ] If nginx already fronts the Python app, no change is needed — same
      `127.0.0.1:8000` upstream + `/collab/` WS upgrade + `X-Real-IP`.

---

## 5. First cutover (migrating from Python)

- [ ] Leave the Python `boatsched.service` **running**; `boatsched-docker` **not** enabled.
- [ ] Run once on the box: `bash /opt/boatsched/deploy/deploy.sh`
      (or push to `main` to let CI do it). It backs up the DB, validates the image
      on `:8001` against a DB copy, stops Python, starts the container on `:8000`,
      and health-gates with auto-rollback to Python on failure.
- [ ] Smoke-test through HTTPS: login, seasons, members progression (read + inline
      level edit), a `/solve`, live `/collab` editing, **and the new eval cards
      (`/api/skills`, `/api/progress` save+reload)**.
- [ ] Soak with Python still installed. When satisfied:
      `sudo systemctl disable boatsched` (keep the unit file for rollback).

---

## What was verified locally vs. what still needs the real box

**Verified locally (this machine):**
- `deploy.sh` / `docker-entrypoint.sh` syntax; both compose files resolve and
  produce the strict prod env (`REQUIRE_AUTH/SECURE_COOKIES/MIN_PASSWORD_LEN`),
  correct ports (`:8000` / `:8001`), isolated validate project + DB dir.
- **Full image build from the fixed repo-root context** — toolchain, OR-Tools SDK
  fetch, full compile, and the in-image `cargo test` gate all pass (incl. the new
  eval tests); `vendor/` correctly excluded. Final image ~201 MB.
- **Container runtime smoke test** (mirrors the `:8001` validation): boots healthy,
  `/api/health` 200, `GET /` serves the embedded SPA, admin login 200 / bad-pw 401
  (DB read + scrypt), `/api/skills` returns the DB-seeded catalog (migrations ran).
- Found + recorded the uid-10001 data-dir permission requirement (§0b).

**NOT testable off the box (verify there):**
- `systemctl` handover and the auto-rollback paths (Python / previous image).
- SSH-from-CI and registry push/pull.
- The end-to-end `deploy.sh` cutover on the live box (the pieces it calls are all
  verified individually above).
