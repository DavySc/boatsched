# boatsched-switch SERVER_PKG — run on the box as `deploy` by deploy/deploy.sh,
# after the new package has been copied into the box's Nix store. Never leaves
# prod broken:
#   1. validate the new server against a COPY of the live DB on :8001,
#   2. back up the live DB, point the profile at the new package, restart,
#   3. health-gate :8000; on failure roll the profile back and restart.

pkg="$1"
profile=/nix/var/nix/profiles/per-user/deploy/boatsched
db=/var/lib/boatsched/boatsched.db
backups=/var/lib/boatsched/backups
keep_backups=20

wait_for_health() {
    local port="$1" timeout="$2" elapsed=0
    until curl -fsS "http://127.0.0.1:${port}/api/health" >/dev/null 2>&1; do
        sleep 2; elapsed=$((elapsed + 2))
        [ "$elapsed" -lt "$timeout" ] || return 1
    done
}

# --- 1. validate on a copy ---------------------------------------------------
workdir="$(mktemp -d)"
pid=""
cleanup() { [ -z "$pid" ] || kill "$pid" 2>/dev/null || true; rm -rf "$workdir"; }
trap cleanup EXIT

[ ! -f "$db" ] || sqlite3 "$db" ".backup '$workdir/boatsched.db'"

set -a
# shellcheck disable=SC1091
. /etc/boatsched/env
set +a
BOATSCHED_REQUIRE_AUTH=1 BOATSCHED_SECURE_COOKIES=1 BOATSCHED_MIN_PASSWORD_LEN=8 \
    "$pkg/bin/boatsched-server" --db "$workdir/boatsched.db" --host 127.0.0.1 --port 8001 \
    >"$workdir/log" 2>&1 &
pid=$!

if ! wait_for_health 8001 30; then
    echo "!! validation: /api/health never came up on :8001 — ABORTING (prod untouched)" >&2
    tail -n 40 "$workdir/log" >&2
    exit 1
fi
curl -fsS http://127.0.0.1:8001/ -o /dev/null \
    || { echo "!! validation: GET / failed — ABORTING (prod untouched)" >&2; exit 1; }
if [ -n "${BOATSCHED_ADMIN:-}" ] && [ -n "${BOATSCHED_ADMIN_PW:-}" ]; then
    body="$(jq -n --arg n "$BOATSCHED_ADMIN" --arg p "$BOATSCHED_ADMIN_PW" '{name: $n, password: $p}')"
    code="$(curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:8001/api/auth/login \
        -H 'Content-Type: application/json' -d "$body" || true)"
    if [ "$code" != "200" ]; then
        echo "!! validation: admin login returned $code — ABORTING (prod untouched)" >&2
        exit 1
    fi
fi
echo "==> validation passed"
cleanup
trap - EXIT

# --- 2. backup, switch, restart -------------------------------------------------
mkdir -p "$backups"
if [ -f "$db" ]; then
    backup="$backups/pre-cutover-$(date +%Y%m%d-%H%M%S).db"
    sqlite3 "$db" ".backup '$backup'"
    echo "==> DB backup: $backup"
    find "$backups" -name 'pre-cutover-*.db' | sort -r | tail -n +$((keep_backups + 1)) | xargs -r rm --
fi
mkdir -p "$(dirname "$profile")"
nix-env -p "$profile" --set "$pkg"
sudo systemctl restart boatsched

# --- 3. health gate + rollback -------------------------------------------------
if wait_for_health 8000 60; then
    echo "==> boatsched healthy on :8000 ($pkg)"
    # Keep a month of generations to roll back to; older ones can be collected.
    nix-env -p "$profile" --delete-generations 30d >/dev/null
    exit 0
fi
echo "!! boatsched did not become healthy — ROLLING BACK" >&2
if nix-env -p "$profile" --rollback; then
    sudo systemctl restart boatsched
    echo "!! rolled back to $(readlink -f "$profile")" >&2
else
    sudo systemctl stop boatsched
    echo "!! no previous generation (first deploy?); service left stopped" >&2
fi
exit 1
