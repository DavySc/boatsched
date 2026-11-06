# Deployment — boatsched on a Hetzner box, built with Nix

The box keeps its distro. Nix builds the server, CI copies it into the box's
Nix store, and a plain systemd unit runs it behind nginx + certbot. There is no
Docker and no build on the box.

| file | what it is |
|---|---|
| `flake.nix` / `flake.lock` | pins nixpkgs (26.05); exposes `boatsched`, `boatsched-switch` and a dev shell |
| `nix/web.nix`, `nix/package.nix` | the SPA and the server; the server build runs `cargo test --workspace` |
| `nix/switch.sh` | `boatsched-switch`, the gated cutover that runs on the box |
| `deploy/deploy.sh` | build → `nix copy` to the box → run `boatsched-switch` |
| `deploy/boatsched.service` | systemd unit; runs the server from a Nix profile |
| `deploy/nginx.conf` | nginx site (hand-installed, see below) |

```
Internet ──HTTPS──▶ nginx (:443, TLS via certbot)        deploy/nginx.conf
                      │  proxy_pass + /collab/ WebSocket upgrade
                      ▼
              boatsched.service (127.0.0.1:8000, user deploy)   ← single process
                      │  ExecStart = /nix/var/nix/profiles/per-user/deploy/boatsched/bin/boatsched-server
              SQLite WAL @ /var/lib/boatsched/boatsched.db
              backups   @ /var/lib/boatsched/backups/ (one per deploy, last 20 kept)
```

The server links nixpkgs' source-built OR-Tools 9.15, and every library it
needs comes with it from the Nix store, so the host's glibc/libstdc++ don't
matter.

**Single process — do not scale.** Sessions, solve jobs and per-schedule `yrs`
docs live in the server's memory. Vertical scaling (bigger box) is the path.

---

## Deploy flow (automatic)

Every push to `main`: the `test` job runs `nix flake check` (which builds the
server, running the whole Rust test suite). If it passes, the `deploy` job runs
`deploy/deploy.sh deploy@$SSH_HOST`:

1. `nix build .#boatsched .#boatsched-switch` (a failing test fails here).
2. `nix copy` both to the box; the box pulls what it can from cache.nixos.org.
3. On the box, `boatsched-switch <new package>`:
   - runs the **new** server on `:8001` against a **copy** of the live DB and
     checks `/api/health`, `GET /` and the bootstrap admin login — any failure
     aborts with prod untouched;
   - backs up the live DB to `/var/lib/boatsched/backups/`;
   - points the `boatsched` profile at the new package and restarts the unit;
   - health-gates `:8000` for 60 s and, on failure, **rolls the profile back to
     the previous generation** and restarts.

By hand, from any machine with Nix and SSH access as `deploy`:

```bash
deploy/deploy.sh deploy@HOST
```

### Rolling back by hand

```bash
P=/nix/var/nix/profiles/per-user/deploy/boatsched
nix-env -p $P --list-generations
nix-env -p $P --rollback            # or --switch-generation N
sudo systemctl restart boatsched
```

Restore a DB backup: `sudo systemctl stop boatsched`, copy a file from
`/var/lib/boatsched/backups/` over `/var/lib/boatsched/boatsched.db` (delete the
`-wal`/`-shm` files), `sudo systemctl start boatsched`.

---

## CI secrets (GitHub → Settings → Secrets and variables → Actions)

Unchanged from the Docker setup:

| Secret | Contents |
|---|---|
| `SSH_PRIVATE_KEY` | Private half of the deploy keypair. |
| `SSH_HOST` | Box hostname or IP. |
| `SSH_USER` | `deploy` |
| `SSH_KNOWN_HOSTS` | Pinned host key line(s) from `ssh-keyscan`. |

---

## One-time setup (migrating the Docker box)

The DB directory, `/etc/boatsched/env` and the `deploy` user stay as they are.
Run as root.

1. **Install Nix** (multi-user) and let `deploy` copy unsigned store paths in:
   ```bash
   sh <(curl -fsSL https://nixos.org/nix/install) --daemon --yes
   echo 'trusted-users = root deploy' >> /etc/nix/nix.conf
   systemctl restart nix-daemon
   install -d -o deploy -g deploy /nix/var/nix/profiles/per-user/deploy
   ```
2. **Install the unit** (don't start it yet):
   ```bash
   cp /opt/boatsched/deploy/boatsched.service /etc/systemd/system/
   systemctl daemon-reload
   ```
   `SOLVER_WORKERS` is still read from `/etc/boatsched/env` (default 2);
   `BUILD_ON_HOST` there is now unused and can go.
3. **Sudoers** — replace the old `boatsched-docker` rules with:
   ```
   deploy ALL=(root) NOPASSWD: /usr/bin/systemctl restart boatsched, /usr/bin/systemctl stop boatsched
   ```
4. **Cut over** (about a minute of downtime): stop the container, then deploy
   once by hand from your machine, then enable the unit for boot:
   ```bash
   ssh root@HOST 'systemctl disable --now boatsched-docker'
   deploy/deploy.sh deploy@HOST
   ssh root@HOST 'systemctl enable boatsched'
   ```
   If that first deploy fails it leaves the unit stopped (there is no previous
   generation to roll back to); `systemctl enable --now boatsched-docker`
   brings the container back while you look.
5. **Clean up** once happy: remove `/etc/systemd/system/boatsched-docker.service`,
   the `boatsched-validate` dirs, the old images (`docker image prune -a`), and
   Docker itself if nothing else uses it. `/opt/boatsched` is no longer needed
   by deploys either.

---

## nginx

nginx and certbot stay hand-managed: `deploy/nginx.conf` is **not** installed by
deploys. After changing it, copy it to `/etc/nginx/conf.d/boatsched.conf` on the
box and run `nginx -t && systemctl reload nginx`.

---

## Day-to-day

```bash
journalctl -u boatsched -f            # app logs
systemctl status boatsched
ls /var/lib/boatsched/backups/
```

- **Secrets** stay in `/etc/boatsched/env` (root:deploy 0640, never in the Nix
  store). Edit, then `sudo systemctl restart boatsched`.
- **Disk:** profile generations older than 30 days are dropped after each
  successful deploy; run `nix-collect-garbage` (as root) now and then, or add a
  weekly timer, to free their store paths.
- **Updating nixpkgs:** `nix flake update`, open a PR; CI builds and tests it.
- **Changing frontend dependencies:** update `npmDepsHash` in `nix/web.nix`
  (`nix run nixpkgs#prefetch-npm-deps web/package-lock.json`), or the build
  fails with the expected hash.
