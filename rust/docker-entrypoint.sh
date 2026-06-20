#!/bin/sh
# Entrypoint for the boatsched Rust server container.
#
# The server reads --workers from argv (the CP-SAT search-worker count, NOT web
# workers — the app is single-process by design). We forward SOLVER_WORKERS from
# the environment so the systemd env file / compose can tune it without changing
# the image. Defaults to 2 to match the 2-vCPU box.
#
# `exec` so the server becomes the (tini-supervised) process that receives
# SIGTERM on `docker compose down` for a clean shutdown.
set -eu

: "${SOLVER_WORKERS:=2}"
: "${BOATSCHED_DB:=/data/boatsched.db}"

exec boatsched-server \
    --db "${BOATSCHED_DB}" \
    --host 0.0.0.0 \
    --port 8000 \
    --workers "${SOLVER_WORKERS}"
