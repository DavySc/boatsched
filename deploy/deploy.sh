#!/usr/bin/env bash
#
# deploy/deploy.sh — run ON THE SERVER over SSH by CI (.forgejo/workflows/deploy.yml).
#
# Idempotent: pull latest main, sync the venv, restart the service. DB migrations
# are NOT a separate step — create_app() runs idempotent migrations at startup
# (server.py:240-247), so `systemctl restart` applies any schema changes.
#
# Assumes one-time provisioning is already done (see deploy/README.md):
#   - /opt/boatsched          repo checkout owned by the `deploy` user
#   - /opt/boatsched/.venv    virtualenv with the package installed
#   - /etc/boatsched/env      secrets + SOLVER_WORKERS (root:deploy, 0640)
#   - /var/lib/boatsched      DB dir owned by `deploy`
#   - scoped sudoers line for `systemctl restart boatsched`

set -euo pipefail

DEPLOY_DIR="${DEPLOY_DIR:-/opt/boatsched}"
VENV="${VENV:-$DEPLOY_DIR/.venv}"
BRANCH="${DEPLOY_BRANCH:-main}"

cd "$DEPLOY_DIR"

echo "==> Fetching origin/$BRANCH"
git fetch --prune origin "$BRANCH"

# Fast-forward only: a diverged prod checkout (e.g. someone committed on the box)
# must hard-fail rather than create a merge commit on production.
echo "==> git pull --ff-only"
git pull --ff-only origin "$BRANCH"

# Sync dependencies into the venv. Production deps only — the dev extra (.[dev])
# is needed solely for the test job in CI, not at runtime, and migrations run at
# startup so no migration tooling is required here.
echo "==> Syncing dependencies into venv"
"$VENV/bin/pip" install --upgrade pip >/dev/null
"$VENV/bin/pip" install -e . >/dev/null

# Restart via the scoped sudoers entry (exactly: systemctl restart boatsched).
echo "==> Restarting boatsched service"
sudo systemctl restart boatsched.service

# Give systemd a moment, then confirm it came back up.
sleep 2
if ! systemctl is-active --quiet boatsched; then
    echo "!! boatsched is not active after restart" >&2
    systemctl --no-pager --lines=20 status boatsched >&2 || true
    exit 1
fi

DEPLOYED_COMMIT="$(git rev-parse HEAD)"
echo "==> Deployed commit: $DEPLOYED_COMMIT"
