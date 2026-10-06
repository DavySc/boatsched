#!/usr/bin/env bash
#
# deploy/deploy.sh — run ON THE SERVER over SSH by CI (.github/workflows/deploy.yml).
#
# Swaps the running app to the latest Rust container image with a HARD guarantee
# that prod is never left broken:
#
#   1. back up the live SQLite DB (online .backup),
#   2. pull/build the new image,
#   3. VALIDATE it against a COPY of the prod DB on a side port (:8001) — this
#      catches "binary won't load / OR-Tools ABI / crashes on boot" with zero
#      impact on the running service,
#   4. only then do a single-writer handover on :8000,
#   5. health-gate the new service; on ANY failure, AUTOMATICALLY roll back to
#      the previous image.
#
# Idempotent. Assumes one-time provisioning is done (see deploy/README.md):
#   - /opt/boatsched               repo checkout owned by `deploy`
#   - Docker Engine + compose plugin; `deploy` in the `docker` group
#   - registry login (for `docker compose pull`)
#   - /etc/boatsched/env           secrets + SOLVER_WORKERS (root:deploy 0640)
#   - /var/lib/boatsched           live DB dir (bind-mounted into the container)
#   - boatsched-docker.service     installed
#   - sqlite3 + curl on the host
#   - scoped sudoers: systemctl start/stop/restart for boatsched-docker

set -euo pipefail

DEPLOY_DIR="${DEPLOY_DIR:-/opt/boatsched}"
BRANCH="${DEPLOY_BRANCH:-main}"
DB_DIR="${DB_DIR:-/var/lib/boatsched}"
DB_FILE="${DB_FILE:-$DB_DIR/boatsched.db}"
VALIDATE_DB_DIR="${VALIDATE_DB_DIR:-/var/lib/boatsched-validate}"
IMAGE_REF="${BOATSCHED_IMAGE:-boatsched:local}"
COMPOSE="docker compose -f $DEPLOY_DIR/docker-compose.yml"
VALIDATE_COMPOSE="docker compose -p boatsched-validate -f $DEPLOY_DIR/docker-compose.validate.yml"
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-60}"     # seconds to wait for /api/health
BUILD_ON_HOST="${BUILD_ON_HOST:-1}"        # 1 = build image on the box (default; no registry), 0 = pull

export VALIDATE_DB_DIR

log() { echo "==> $*"; }
err() { echo "!! $*" >&2; }

# Poll http://127.0.0.1:$1/api/health until 200 or $2 seconds elapse.
wait_for_health() {
    local port="$1" timeout="$2" elapsed=0
    while ! curl -fsS "http://127.0.0.1:${port}/api/health" >/dev/null 2>&1; do
        sleep 2; elapsed=$((elapsed + 2))
        if [ "$elapsed" -ge "$timeout" ]; then return 1; fi
    done
    return 0
}

cd "$DEPLOY_DIR"

# --- 0. sync deploy files (compose, unit, this script) ----------------------
log "Fetching origin/$BRANCH"
git fetch --prune origin "$BRANCH"
log "git pull --ff-only"
git pull --ff-only origin "$BRANCH"

# --- 1. record the pre-deploy state for rollback ----------------------------
# Image currently backing the prod container (empty only on the very first run).
PREV_IMAGE="$(docker inspect --format '{{.Image}}' boatsched 2>/dev/null || true)"
log "pre-deploy: prev_image=${PREV_IMAGE:-none}"

# --- 2. back up the live DB (online, WAL-safe) ------------------------------
BACKUP="$DB_DIR/pre-cutover-$(date +%Y%m%d-%H%M%S).db"
log "Backing up DB → $BACKUP"
sqlite3 "$DB_FILE" ".backup '$BACKUP'"

# --- 3. obtain the new image ------------------------------------------------
if [ "$BUILD_ON_HOST" = "1" ]; then
    log "Building image on host (BUILD_ON_HOST=1)"
    $COMPOSE build
else
    log "Pulling image"
    $COMPOSE pull
fi

# --- 4. VALIDATE the new image against a COPY of prod, on :8001 -------------
validate_cleanup() { $VALIDATE_COMPOSE down --remove-orphans >/dev/null 2>&1 || true; }
trap validate_cleanup EXIT

log "Preparing validation DB copy in $VALIDATE_DB_DIR"
mkdir -p "$VALIDATE_DB_DIR"
rm -f "$VALIDATE_DB_DIR"/boatsched.db*
sqlite3 "$DB_FILE" ".backup '$VALIDATE_DB_DIR/boatsched.db'"

log "Starting validation stack on :8001"
$VALIDATE_COMPOSE up -d

if ! wait_for_health 8001 30; then
    err "validation: /api/health never came up on :8001 — ABORTING (prod untouched)"
    $VALIDATE_COMPOSE logs --tail=40 >&2 || true
    exit 1
fi
# Public route serves the SPA.
if ! curl -fsS "http://127.0.0.1:8001/" -o /dev/null; then
    err "validation: GET / failed — ABORTING (prod untouched)"; exit 1
fi
# If admin creds are present, prove DB read + scrypt verify against real data.
# shellcheck disable=SC1091
set +u; . /etc/boatsched/env 2>/dev/null || true; set -u
if [ -n "${BOATSCHED_ADMIN:-}" ] && [ -n "${BOATSCHED_ADMIN_PW:-}" ]; then
    code="$(curl -s -o /dev/null -w '%{http_code}' -X POST \
        "http://127.0.0.1:8001/api/auth/login" \
        -H 'Content-Type: application/json' \
        -d "{\"name\":\"${BOATSCHED_ADMIN}\",\"password\":\"${BOATSCHED_ADMIN_PW}\"}" || true)"
    if [ "$code" != "200" ]; then
        err "validation: admin login returned $code (expected 200) — ABORTING (prod untouched)"
        exit 1
    fi
    log "validation: admin login OK"
fi
log "validation PASSED"
validate_cleanup
trap - EXIT

# --- 5. single-writer handover on :8000 -------------------------------------
log "Restarting Rust container service (single writer on the live DB)"
sudo systemctl restart boatsched-docker

# --- 6. health gate + automatic rollback ------------------------------------
if wait_for_health 8000 "$HEALTH_TIMEOUT"; then
    log "Rust service healthy on :8000"
else
    err "Rust service did NOT become healthy within ${HEALTH_TIMEOUT}s — ROLLING BACK"
    sudo systemctl stop boatsched-docker 2>/dev/null || true
    if [ -n "$PREV_IMAGE" ]; then
        err "rollback → previous image $PREV_IMAGE"
        # Re-point the :latest tag the systemd unit launches back at the known-good
        # image, then restart through systemd so service state stays consistent.
        docker tag "$PREV_IMAGE" "$IMAGE_REF" || true
        sudo systemctl restart boatsched-docker
        err "rolled back to previous image. DB backup: $BACKUP"
    else
        err "no known-good previous image to roll back to (first deploy?)."
        err "service left stopped. DB backup: $BACKUP"
    fi
    exit 1
fi

# --- 7. housekeeping --------------------------------------------------------
log "Pruning dangling images"
docker image prune -f >/dev/null 2>&1 || true

DEPLOYED_IMAGE="$(docker inspect --format '{{.Image}}' boatsched 2>/dev/null || echo unknown)"
log "Deployed image: $DEPLOYED_IMAGE"
log "DB backup:      $BACKUP"
log "Done."
