#!/usr/bin/env bash
# Host driver for the pnl-packages install + example sweep.
#
# Usage:
#   tests/docker-sweep/sweep.sh <alpine|ubuntu|both> [pkg ...]
#
# For each distro it: builds the toolchain image, builds pnl ONCE (cached target
# volume), then runs the whole package sweep inside ONE long-lived container in the
# foreground with internal parallelism (PAR jobs). `both` runs the two distros
# concurrently, so PAR=4 per distro = 8 parallel jobs total.
#
# Env:
#   PAR=4            jobs per distro (default 4)
#   NO_BINUTILS=1    remove `nm` in-container so the export filter must use the
#                    in-binary parser (proves binutils is not a requirement)
#   PNL_SRC          pnl checkout (default: repo root, derived from this script)
#   PKG_SRC          pnl-packages checkout (default: ../pnl-packages beside pnl)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PNL_SRC="${PNL_SRC:-$(cd "$HERE/../.." && pwd)}"
PKG_SRC="${PKG_SRC:-$(cd "$PNL_SRC/.." && pwd)/pnl-packages}"
PAR="${PAR:-4}"
NO_BINUTILS="${NO_BINUTILS:-1}"

[ -d "$PKG_SRC/packages" ] || { echo "pnl-packages not found at $PKG_SRC (set PKG_SRC)"; exit 1; }

run_distro() { # $1=distro, rest=packages
  local distro="$1"; shift
  local img="pnl-sweep:$distro"
  docker build -t "$img" -f "$HERE/Dockerfile.$distro" "$HERE" >/dev/null
  # A fresh snapshot path per run — Docker Desktop's VM caches a mount path's
  # contents, so a brand-new path always reflects just-edited manifests.
  local snap="$HERE/results/_pkgsrc-$distro-$$"
  rm -rf "$snap"; mkdir -p "$snap"
  rsync -a --exclude '.git' "$PKG_SRC/" "$snap/"
  mkdir -p "$HERE/results/$distro"

  local common=(
    --rm
    -v "$PNL_SRC":/src/pnl
    -v "$snap":/src/pnl-packages:ro
    -v "$HERE/results":/work/results
    -v "$HERE/in-container.sh":/in-container.sh:ro
    -v "pnl-sweep-cargo-$distro":/cache/cargo
    -v "pnl-sweep-target-$distro":/cache/target
    -e "DISTRO=$distro" -e "PAR=$PAR" -e "NO_BINUTILS=$NO_BINUTILS"
  )
  # 1) build pnl once, 2) run the sweep in the foreground of one container.
  docker run "${common[@]}" -e MODE=build "$img" bash /in-container.sh
  docker run "${common[@]}" -e MODE=sweep "$img" bash /in-container.sh "$@"
  rm -rf "$snap"
}

case "${1:?usage: sweep.sh <alpine|ubuntu|both> [pkg ...]}" in
  alpine) shift; run_distro alpine "$@" ;;
  ubuntu) shift; run_distro ubuntu "$@" ;;
  both)
    shift
    run_distro alpine "$@" & a=$!
    run_distro ubuntu "$@" & u=$!
    wait $a; wait $u
    ;;
  *) echo "usage: sweep.sh <alpine|ubuntu|both> [pkg ...]"; exit 1 ;;
esac
