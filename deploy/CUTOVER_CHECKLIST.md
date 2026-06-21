# Cutover runbook — live Python box → Rust container

**Scenario this guide covers:** the **Python** boatsched is **already running in
production with live data**, and you want to switch it to the Rust container with
no data loss and no extended downtime.

> Setting up a **clean box** from scratch is a separate concern — see
> [`deploy/README.md` → One-time server provisioning](./README.md#one-time-server-provisioning).
> This runbook does **not** repeat it; it only covers the delta from a
> Python-serving box to the container.

---

## Starting state (already true because Python is live)

These exist and are **reused unchanged** — do not recreate them:

- `deploy:deploy` user; repo checked out at `/opt/boatsched`.
- Live SQLite DB at `/var/lib/boatsched/boatsched.db` (+ `-wal`/`-shm`), owned by
  `deploy`. **This is the real data — it is reused in place, byte-for-byte.**
- `/etc/boatsched/env` with `BOATSCHED_ADMIN`, `BOATSCHED_ADMIN_PW`,
  `SOLVER_WORKERS` (the container reads the same file).
- nginx + certbot terminating TLS, proxying to `127.0.0.1:8000`. **No nginx change**
  — the container listens on the same address and speaks the same `/collab/` WS.
- `boatsched.service` (Python) running. **Leave it running** until the handover.

The Rust DB layer uses `CREATE TABLE IF NOT EXISTS` + the same idempotent
migrations as Python, in WAL mode, with byte-compatible scrypt hashes — so no
schema change, no data migration, no forced re-login.

---

## Step 1 — Code is ready on `main`

The deploy builds from `main` (CI trigger + `deploy.sh` does `git pull --ff-only
origin main`).

- [ ] The Docker build fixes are merged to `main` (build context, `user:`, etc.
      — already committed on `rust-rewrite`; merge that branch).
- [ ] `main` builds the image green (CI, or a local `docker build -f rust/Dockerfile .`).
      Verified locally already: the build passes the in-image `cargo test` gate and
      the container boots + serves + logs in.

---

## Step 2 — One-time delta on the live box (Python stays up the whole time)

None of this touches the running Python service or its DB. Run on the box.

- [ ] **Confirm the deploy uid/gid is `1000:1000`** — `id deploy`. The compose
      files set `user: "1000:1000"` so the container runs as `deploy` and can open
      the live deploy-owned DB. **If it differs, edit the `user:` line in
      `docker-compose.yml` AND `docker-compose.validate.yml` to match.**
- [ ] Install Docker + compose plugin; add `deploy` to the `docker` group:
      ```bash
      sudo apt install -y docker.io docker-compose-plugin
      sudo systemctl enable --now docker
      sudo usermod -aG docker deploy      # re-login / new shell for deploy to take effect
      ```
- [ ] Update the checkout: `sudo -u deploy git -C /opt/boatsched pull --ff-only origin main`
      (brings in `docker-compose.yml`, the unit, `deploy.sh`).
- [ ] Create the validation DB dir (owned by deploy):
      `sudo install -d -o deploy -g deploy /var/lib/boatsched-validate`
- [ ] Pulling the image (pick one):
      - **Registry (default):** `sudo -u deploy docker login codeberg.org` (token with
        `read:package`).
      - **Build on the box (no registry):** set `BUILD_ON_HOST=1` in
        `/etc/boatsched/env` so `deploy.sh` runs `docker compose build` instead.
- [ ] Install the container unit, **but do NOT enable/start it** (the cutover does
      the handover):
      ```bash
      sudo cp /opt/boatsched/deploy/boatsched-docker.service /etc/systemd/system/
      sudo systemctl daemon-reload
      ```
- [ ] Scoped sudoers so `deploy` can start/stop **both** services during
      handover/rollback — see the exact block in
      [README §8](./README.md#one-time-server-provisioning). Verify with `visudo -cf`.

> `/etc/boatsched/env` is reused as-is. Just confirm it still has
> `BOATSCHED_ADMIN` + `BOATSCHED_ADMIN_PW` (the validation step logs in with them).

---

## Step 3 — (Only if deploying via CI) Forgejo secrets

Skip this if you'll run `deploy.sh` by hand (Step 4, option B).

- [ ] `REGISTRY_USER`, `REGISTRY_TOKEN` (`write:package`), `SSH_PRIVATE_KEY`,
      `SSH_HOST`, `SSH_USER=deploy`, `SSH_KNOWN_HOSTS` (from `ssh-keyscan`).
- [ ] Authorize the CI deploy **public** key in `~deploy/.ssh/authorized_keys`.
- [ ] Confirm the registry path/casing (`codeberg.org/oventosti/boatsched`) matches
      the real package.

---

## Step 4 — Run the cutover

`deploy.sh` is **never-break-prod**: backup live DB → pull/build image → validate
it on `:8001` against a **copy** of the live DB (health + `GET /` + admin login) →
stop Python (releases the DB) → start the container on `:8000` → health-gate, with
**automatic rollback to Python** on any failure.

Pick one:

- **A — via CI:** push to `main`. The build job builds+tests+pushes the image; the
  deploy job SSHes in and runs `deploy.sh`.
- **B — manually on the box:**
  ```bash
  sudo -u deploy bash /opt/boatsched/deploy/deploy.sh
  # (BUILD_ON_HOST=1 already in the env file if you're not using the registry)
  ```

- [ ] Watch the log: it must print `validation PASSED` then `Rust service healthy
      on :8000`. If validation fails, **prod is untouched** (Python still serving).
      If the post-handover health check fails, it **auto-rolls back to Python**.

---

## Step 5 — Verify, soak, then retire Python

- [ ] Smoke-test through **HTTPS** (real domain): login, seasons, members
      progression (read + inline level edit), a `/solve`, live `/collab` editing,
      and the **eval cards** (open a student card, set skipper + skills, save,
      reload — confirm it persists; no 404 in DevTools).
- [ ] Confirm the live data is all there (members, schedules, seasons).
- [ ] Soak with Python still installed (it's stopped, not removed).
- [ ] Once satisfied, stop it from restarting but **keep the unit for rollback**:
      `sudo systemctl disable boatsched`

---

## Rollback

- **Automatic:** `deploy.sh` rolls back to Python (or the previous image) if the
  new container fails its health gate. No action needed.
- **Manual → Python:**
  ```bash
  sudo systemctl stop boatsched-docker
  sudo systemctl start boatsched
  ```
- **Manual → previous image:**
  ```bash
  docker tag <previous-image-id> codeberg.org/oventosti/boatsched:latest
  sudo systemctl restart boatsched-docker
  ```
- **Data:** the most recent `/var/lib/boatsched/pre-cutover-*.db` (written by
  `deploy.sh` before handover) is a point-in-time restore of the DB.

---

## What was verified locally vs. what to confirm on the box

**Verified locally (this machine):**
- `deploy.sh` / entrypoint syntax; both compose files resolve to the strict prod
  env, correct ports (`:8000`/`:8001`), isolated validate project + DB dir.
- **Full image build** from the repo-root context — toolchain + OR-Tools SDK +
  compile + the in-image `cargo test` gate all pass (incl. the new eval tests).
- **Container runtime**: boots healthy, `/api/health` 200, serves the SPA, admin
  login 200 / bad-pw 401 (DB read + scrypt), `/api/skills` returns the seeded
  catalog (migrations ran), and it runs fine **as a non-image uid** against a DB
  dir it doesn't own by image-uid — confirming the `user: "1000:1000"` approach.

**Confirm on the live box (can't be tested off-box):**
- `id deploy` is `1000:1000` (else adjust the `user:` line — Step 2).
- The `systemctl` handover + auto-rollback paths.
- SSH-from-CI / registry pull (if using CI).
- The real `deploy.sh` cutover end-to-end (its individual pieces are verified above).
