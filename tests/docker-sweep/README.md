# pnl-packages Docker sweep

Installs every package in the sibling `pnl-packages` checkout and runs each one's
`EXAMPLES.md` sample through PHP FFI, on **Alpine (musl)** and **Ubuntu (glibc)**, to
catch generator/runtime regressions across the real corpus. `pnl install`'s only
external toolchain requirement is **libclang**; the sweep removes `nm` by default
(`NO_BINUTILS=1`) to prove binutils is not needed.

## Run

```sh
# Both distros concurrently, 4 jobs each = 8 parallel jobs. Builds pnl once per
# distro (cached), then sweeps inside one foreground container per distro.
tests/docker-sweep/sweep.sh both

# One distro, or a subset of packages:
tests/docker-sweep/sweep.sh alpine
tests/docker-sweep/sweep.sh ubuntu libsdl libquickjs libncurses
```

Env: `PAR` (jobs per distro, default 4), `NO_BINUTILS` (default 1),
`PNL_SRC` / `PKG_SRC` (checkout paths; default the repo and `../pnl-packages`).

## Results

Per distro under `results/<distro>/`:
`status/<pkg>.tsv` (install), `example-status/<pkg>.tsv` (example run), and the full
`logs/` + `example-logs/`. The container prints an install/example PASS·FAIL summary
at the end. (`results/` is git-ignored.)

## Reading outcomes (agent rules)

When driving this sweep, evaluate results in this order — do **not** treat raw
example-FAIL counts as the bottom line:

1. **Install PASS count** is the primary signal. The only acceptable install FAILs
   are libraries the distro does not package (skipped by the resolver). A new
   install FAIL is a real regression.
2. **Load-time errors are the regression signal.** Grep example logs for
   `Failed resolving C function`, `ParserException`, `Undefined C type`,
   `Failed loading`, `file too short`. These mean a generated cdef broke — this
   count **must be 0**. Anything here is a generator regression; investigate before
   anything else.
3. **Post-load example FAILs are NOT automatically regressions.** A log that prints
   real output then exits non-zero is usually one of:
   - a **service** dependency (libpq → a DB, libhiredis → a Redis server);
   - **hardware** (libhidapi → an HID device, libserialport → a serial port,
     librtlsdr → an SDR, libsdl/libvulkan → a real GPU/display beyond the dummy
     drivers the harness sets);
   - a **stale EXAMPLES.md** (e.g. calling a `static inline` function with no
     exported symbol, or freeing a string pnl already copied to a PHP string).
   Triage each by reading its log; fix the example or document the limitation. Do
   not "fix" the harness to paper over a real binding gap.
4. **Compare against the baseline**, not zero. The achievable floor is "every
   installable, non-hardware, non-service package passes". As of the last full run:
   Alpine 138/144 install, 138/138 installable examples PASS; Ubuntu 141/144
   install, only hardware-gated libhidapi/libserialport remain.

## Speed/isolation tradeoff

A single foreground container per distro shares apt/apk state across packages (one
index update; deps accumulate) — faster, but a loose `checkIfExists` in one package
could in principle mask a missing dev package in another. For a final
release-gating run where strict per-package clean state matters, run packages in
smaller batches or separate invocations.
