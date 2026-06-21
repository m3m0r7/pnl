#!/usr/bin/env bash
# Runs INSIDE a sweep container. Two phases driven by $MODE:
#   MODE=build  -> build pnl once from /src/pnl into the cached target volume.
#   MODE=sweep  -> install every package and run its EXAMPLES.md sample, PAR jobs at
#                  a time, IN THE FOREGROUND of this one long-lived container (no
#                  per-package container churn), writing one result file per package.
#
# Speed/isolation note: a single container shares apt/apk state across packages
# (faster: one index update, deps accumulate). That trades the strict per-package
# clean-state isolation of the old container-per-package model for wall-clock; it is
# the right default for a full sweep. Set NO_BINUTILS=1 to remove `nm` so the export
# filter must use the in-binary parser (proves binutils is not required).
set -u

DISTRO="${DISTRO:?DISTRO required}"
MODE="${MODE:?MODE required}"
PAR="${PAR:-4}"
SRC=/src/pnl
PKGROOT=/src/pnl-packages/packages
RESULTS="/work/results/$DISTRO"

export CARGO_HOME=/cache/cargo
export CARGO_TARGET_DIR=/cache/target
export RUSTUP_HOME=/opt/rustup
export PATH=/opt/cargo/bin:$PATH
export DEBIAN_FRONTEND=noninteractive

LC=$(find /usr/lib /usr/lib64 -maxdepth 3 -name 'libclang*.so*' 2>/dev/null | head -1)
[ -n "$LC" ] && export LIBCLANG_PATH="$(dirname "$LC")"
# musl static binaries cannot dlopen() libclang at runtime.
if command -v apk >/dev/null 2>&1; then
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=-crt-static"
fi

PNL="$CARGO_TARGET_DIR/release/pnl"

if [ "$MODE" = "build" ]; then
  echo "== [$DISTRO] building pnl =="
  ( cd "$SRC" && cargo build --release --bins ) > "$RESULTS/build.log" 2>&1
  rc=$?
  if [ $rc -ne 0 ] || [ ! -x "$PNL" ]; then
    echo "!! pnl build FAILED (rc=$rc)"; tail -40 "$RESULTS/build.log"; exit 1
  fi
  echo "== built: $($PNL --version 2>&1 | head -1) =="
  exit 0
fi

# ---- MODE=sweep ----
mkdir -p "$RESULTS/logs" "$RESULTS/example-logs" "$RESULTS/status" "$RESULTS/example-status"
rm -f "$RESULTS"/status/*.tsv "$RESULTS"/example-status/*.tsv 2>/dev/null || true

# Optionally simulate a host without binutils (the reported Ubuntu 24.04 case).
if [ "${NO_BINUTILS:-}" = 1 ]; then
  rm -f "$(command -v nm 2>/dev/null)" /usr/bin/nm /usr/local/bin/nm 2>/dev/null || true
fi

# One index update for the whole sweep (shared across packages).
if command -v apt-get >/dev/null 2>&1; then apt-get update -y >/dev/null 2>&1 || true; fi
if command -v apk     >/dev/null 2>&1; then apk update        >/dev/null 2>&1 || true; fi

# Serialize the system package manager across the PAR parallel jobs. Concurrent
# `pnl install` native-dep steps (`apk add` / `apt-get install`) otherwise collide on
# the apk/dpkg lock ("Failed to open apk database / Could not get lock"). REPLACE the
# real binary in place with a flock wrapper (not just a PATH shim): Ubuntu packages
# invoke `sudo apt-get`, and sudo resolves via secure_path, bypassing PATH. Only the
# package-manager step serializes — libclang parse, generation, example runs stay
# fully parallel. The container is ephemeral (--rm), so mutating the binary is safe.
for mgr in apk apt-get; do
  real=$(command -v "$mgr" 2>/dev/null) || continue
  if [ ! -e "$real.real" ]; then
    mv "$real" "$real.real"
    cat > "$real" <<EOF
#!/bin/sh
exec flock /tmp/pnl-pkgmgr.lock "$real.real" "\$@"
EOF
    chmod +x "$real"
  fi
done

SCHEMA=$(sed -n 's/.*"schema_version"[: ]*"\([^"]*\)".*/\1/p' "$PKGROOT/libc/pnlx.json" | head -1)

one_package() { # $1 = package name
  local PKG="$1" d="$PKGROOT/$1"
  [ -f "$d/pnlx.json" ] || { printf '%s\t%s\t%s\n' "$PKG" SKIP 2 > "$RESULTS/status/$PKG.tsv"; return; }
  local proj="/tmp/proj/$PKG"; rm -rf "$proj"; mkdir -p "$proj"
  cat > "$proj/pnl.json" <<EOF
{ "schema_version": "$SCHEMA", "repositories": [], "load_paths": [], "output_dir": "@pnlx",
  "features": { "use_functions": true, "allow_cdata": true, "use_php_scalars_in_params": true },
  "extensions": {} }
EOF
  local log="$RESULTS/logs/$PKG.log"
  ( cd "$proj" && timeout 900 "$PNL" install "$d" -y --allow-unverified-install-scripts ) > "$log" 2>&1
  local rc=$?
  local status; [ $rc -eq 0 ] && status=PASS || status=FAIL
  printf '%s\t%s\t%s\n' "$PKG" "$status" "$rc" > "$RESULTS/status/$PKG.tsv"

  # Run the first ```php block of EXAMPLES.md through PHP FFI.
  local elog="$RESULTS/example-logs/$PKG.log"
  if [ "$status" != PASS ]; then
    printf '%s\t%s\t%s\n' "$PKG" "SKIP(install-failed)" 2 > "$RESULTS/example-status/$PKG.tsv"; return
  fi
  if [ ! -f "$d/EXAMPLES.md" ]; then
    printf '%s\t%s\t%s\n' "$PKG" "SKIP(no-examples)" 2 > "$RESULTS/example-status/$PKG.tsv"; return
  fi
  awk '/^```php/{f=1;next} /^```/{if(f)exit} f' "$d/EXAMPLES.md" > "$proj/body.php"
  # Modern EXAMPLES.md are self-contained (`<?php` + own autoload require); older
  # ones were bare bodies. Prepend `<?php` only when absent (never double up).
  if head -1 "$proj/body.php" | grep -q '^<?php'; then
    cp "$proj/body.php" "$proj/example.php"
  else
    { echo '<?php'; cat "$proj/body.php"; } > "$proj/example.php"
  fi
  # Headless drivers so display/audio libs run without an X server or sound card.
  export SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy SDL_RENDER_DRIVER=software
  export PA_ALSA_PLUGHW=1 AUDIODEV=null
  export XDG_RUNTIME_DIR="$proj/.xdg"; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
  php -d ffi.enable=1 -d auto_prepend_file="$proj/@pnlx/autoload.php" "$proj/example.php" > "$elog" 2>&1
  local erc=$?
  local estatus; [ $erc -eq 0 ] && estatus=PASS || estatus=FAIL
  printf '%s\t%s\t%s\n' "$PKG" "$estatus" "$erc" > "$RESULTS/example-status/$PKG.tsv"
}

# Package list (args override; otherwise every package).
if [ "$#" -gt 0 ]; then PKGS=("$@"); else
  PKGS=(); for dd in "$PKGROOT"/*/; do [ -f "$dd/pnlx.json" ] && PKGS+=("$(basename "$dd")"); done
fi
echo "== [$DISTRO] sweeping ${#PKGS[@]} package(s), PAR=$PAR, NO_BINUTILS=${NO_BINUTILS:-0} =="

running=0
for p in "${PKGS[@]}"; do
  one_package "$p" &
  running=$((running+1))
  if [ "$running" -ge "$PAR" ]; then wait -n 2>/dev/null || wait; running=$((running-1)); fi
done
wait

# Aggregate.
inst_pass=$(cat "$RESULTS"/status/*.tsv 2>/dev/null | awk -F'\t' '$2=="PASS"' | wc -l | tr -d ' ')
inst_fail=$(cat "$RESULTS"/status/*.tsv 2>/dev/null | awk -F'\t' '$2=="FAIL"' | wc -l | tr -d ' ')
ex_pass=$(cat "$RESULTS"/example-status/*.tsv 2>/dev/null | awk -F'\t' '$2=="PASS"' | wc -l | tr -d ' ')
ex_fail=$(cat "$RESULTS"/example-status/*.tsv 2>/dev/null | awk -F'\t' '$2=="FAIL"' | wc -l | tr -d ' ')
echo "== [$DISTRO] install: $inst_pass PASS / $inst_fail FAIL | example: $ex_pass PASS / $ex_fail FAIL =="
echo "[$DISTRO] install FAIL: $(cat "$RESULTS"/status/*.tsv 2>/dev/null | awk -F'\t' '$2=="FAIL"{print $1}' | tr '\n' ' ')"
echo "[$DISTRO] example FAIL: $(cat "$RESULTS"/example-status/*.tsv 2>/dev/null | awk -F'\t' '$2=="FAIL"{print $1}' | tr '\n' ' ')"
