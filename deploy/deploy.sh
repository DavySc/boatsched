#!/usr/bin/env bash
#
# deploy/deploy.sh deploy@HOST — run by CI (.github/workflows/deploy.yml) or by
# hand from any machine with Nix.
#
# Builds the server (its build runs `cargo test --workspace`, so a failing test
# fails the deploy) and the cutover script, copies both into the box's Nix
# store, and runs the cutover there (nix/switch.sh): validate on a copy of the
# live DB, back up, switch, health-gate, roll back on failure.
set -euo pipefail

target="${1:?usage: deploy/deploy.sh deploy@host}"
cd "$(dirname "$0")/.."

mapfile -t paths < <(nix build --no-link --print-out-paths -L .#boatsched .#boatsched-switch)
pkg="${paths[0]}" switch="${paths[1]}"
echo "==> built $pkg"

# -s: let the box fetch what cache.nixos.org has (OR-Tools etc.) instead of
# uploading it. remote-program: a non-interactive SSH shell may not have Nix on
# PATH. NIX_SSHOPTS carries CI's key/known_hosts to nix copy and ssh.
nix copy -s --to "ssh://$target?remote-program=/nix/var/nix/profiles/default/bin/nix-store" \
    "$pkg" "$switch"
# shellcheck disable=SC2086
ssh ${NIX_SSHOPTS:-} "$target" "$switch/bin/boatsched-switch" "$pkg"
