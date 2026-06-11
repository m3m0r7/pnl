# Commands

[← Documentation index](../../README.md) · [日本語](../ja/commands.md)

## pnl Commands

### `pnl help`

Prints the command summary and the available subcommands.

```sh
pnl help
```

### `pnl version`

Prints the `pnl` version.

```sh
pnl version
```

Example output:

```text
0.1.0
```

### `pnl -i` / `pnl --information`

Prints a neofetch-style banner: the pnl ASCII-art logo next to the version, OS and architecture, host, binary location, repository URLs, license, copyright, and the extensions installed in the current workspace with their install locations. `pnlx -i` / `pnlx --information` prints the same with the pnlx logo.

```sh
pnl -i
```

Example output:

```text
  ██████╗ ███╗   ██╗██╗        pnl 0.1.5
  ██╔══██╗████╗  ██║██║        ─────────
  ██████╔╝██╔██╗ ██║██║        OS:         macos (aarch64)
  ██╔═══╝ ██║╚██╗██║██║        Host:       mymachine.local
  ██║     ██║ ╚████║███████╗   Binary:     /usr/local/bin/pnl
  ╚═╝     ╚═╝  ╚═══╝╚══════╝   Repository: https://github.com/m3m0r7/pnl
                               Packages:   https://github.com/m3m0r7/pnl-packages
                               License:    MIT (run `pnl --license` for details)
                               Copyright:  Copyright (c) 2026 memory
                               Extensions: libsdl/libsdl 2.32.10 (./@pnlx/packages/libsdl/libsdl/2.32.10)
```

### `pnl -l` / `pnl --license`

Prints the LICENSE file verbatim (it is embedded into the binary at build time), followed by the third-party components that require attribution: the runtime Rust crates, the bundled/dynamically loaded native libraries (vendored libgit2 and OpenSSL, runtime-loaded libclang), and the PHP SDK's runtime composer packages. `pnlx -l` / `pnlx --license` prints the same.

```sh
pnl --license
```

### `pnl init`

Creates a default `pnl.json` if one doesn't already exist.

```sh
pnl init
```

Example output:

```text
initialized ./pnl.json
```

What gets created:

```jsonc
{
  // Schema version for validating pnl.json.
  "schema_version": "2026-07-01",
  // No repositories configured initially.
  "repositories": [],
  // No extra C library folders configured initially.
  "load_paths": [],
  // Global function generation is off initially.
  "features": {
    "use_functions": false
  },
  // No extensions installed yet.
  "extensions": {}
}
```

### `pnl install <source>`

Installs the given extension. Specifically, it finds the C library and headers, generates the PHP/Rust wrappers, compiles the bridge, and updates `pnl.json`, the lockfile, the pathmap, and `@pnlx/autoload.php`.

The source can be a URL, a local path, a **bare package name**, or a **distribution archive** (`.tar.gz`/`.tgz`/`.zip`, local or remote — it is downloaded if needed, extracted, and must contain a `pnlx.json`). You can pass **several sources at once** (`pnl install libusb libnfc`).

A bare name is resolved against your configured `repositories`, highest [`priority`](configuration.md#writing-pnljson) first, falling back to the built-in default repository `https://github.com/m3m0r7/pnl-packages` (kept internally at priority 0 — it is *not* written into `pnl.json`). Append `@<version>` to pin a version — for git sources that checks out the matching tag/branch, and the resolved package version must match. Running `pnl install` with **no source** restores every extension from the lockfile at its locked version, re-verifying each package's content against the recorded sha256.

If the package declares an `installation` recipe for your OS or Linux distro, `pnl install` offers to run it (e.g. `brew install …`) before resolving the native library, skipping it when the package's `checkIfExists` check already passes. Pass `-y` / `--yes` to accept that prompt automatically (or `-n` / `--no-interaction` to take the default). On Linux the recipe is selected from `/etc/os-release`: the distro `ID` (e.g. `alpine`, `ubuntu`, `fedora`) is tried first, then each `ID_LIKE` ancestor (e.g. `debian`, `rhel`), then a generic `linux` key. If the install commands fail, pnl reports which command failed and asks you to install the libraries and headers manually before running `pnl install` again.

Two flags adjust the generated PHP:

- `--alias-class <Class>` additionally exposes the extension under `<Class>` via `class_alias`, keeping the original class.
- `--function-prefix <prefix>` prepends `<prefix>` to every generated function and method name (the unprefixed names are not kept).

```sh
# Install libusb by bare name (resolved against the default repository).
pnl install libusb

# Or from an explicit URL, optionally pinned to a version.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install libusb@1.0.29

# Restore everything from the lockfile (with sha256 verification).
pnl install
```

Example output:

```text
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
  ✓ resolved libusb-1.0 1.0.27 libusb-1.0.dylib
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/libusb.ffi.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/Libusb.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/LibusbContext.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/index.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/functions.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/function.aliases.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/libusb.bridge.rs
  ✓ installed extension libusb/libusb

added 1 extension in 1.42s
```

The main files produced:

```text
@pnlx/packages/libusb/libusb/1.0.27/
@pnlx/packages/libusb/libusb/1.0.27/bridge/libusb_bridge.dylib
pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
```

#### Content integrity

When a package is installed, `pnl` computes a content signature for it: every file is hashed with sha256 (in sorted order), and those per-file hashes are hashed together into one digest, which is recorded as `dist.sha256` in `pnlx-lock.json`. On a later install of the **same version**, the freshly downloaded content is hashed again and compared to the locked digest; if they differ, the install is aborted with an integrity error because the content was modified or tampered with. Installing a new version is treated as a legitimate update. (Generated output, `.git`, and the workspace directory are excluded from the digest.)

### `pnl update [vendor/package]`

Reinstalls one package — or every installed package — from the source recorded in the lockfile.

```sh
# Reinstall just libusb from its recorded source.
pnl update libusb/libusb

# Reinstall every installed extension.
pnl update
```

This command:

- reads `pnlx-lock.json`,
- reuses each recorded `source.url`,
- and re-runs install, generation, bridge compilation, and pathmap updates.

### `pnl uninstall <vendor/package>`

Removes an extension from `pnl.json` and the lockfile, deletes its installed folder, and regenerates `@pnlx/autoload.php`.

```sh
# Remove libusb from pnl.json, the lock/pathmap, and @pnlx/packages.
pnl uninstall libusb/libusb
```

Example output:

```text
uninstalled libusb/libusb
```

### `pnl list [glob]`

Lists installed extensions. `pnl list` is the same as `pnl list extensions`. Pass a glob pattern (`*` and `?`) to filter; it matches both the full `vendor/extension` name and its leaf, so `pnl list 'lib*'` finds `acme/libusb`.

```sh
pnl list
pnl list extensions
pnl list 'lib*'
```

Example output:

```text
libnfc/libnfc 1.8.0 1.8.0
libsdl/libsdl 2.32.10 2.32.10
libusb/libusb 1.0.29 1.0.29
```

### `pnl find [glob]`

Lists the packages **available** from your configured `repositories` plus the built-in default repository, optionally filtered by a glob pattern. Like `pnl list`, the pattern matches the full `vendor/extension` name or its leaf.

Each repository is enumerated cheaply when it publishes a `repository-index.json` (fetched over HTTP for GitHub/`https` repositories, or read from disk for `file` ones); otherwise pnl falls back to a shallow clone and a directory walk. When two repositories offer the same package, the higher-[`priority`](configuration.md#writing-pnljson) one wins (the default repository is consulted last).

```sh
# Browse everything the default repository offers.
pnl find

# Only packages whose name starts with "lib".
pnl find 'lib*'
```

Example output (name, available versions, source repository):

```text
libusb/libusb 1.0.29 https://github.com/m3m0r7/pnl-packages/tree/main/packages
libuv/libuv 1.48.0 https://github.com/m3m0r7/pnl-packages/tree/main/packages
```

### `pnl list native`

Lists the C libraries that were found, based on `@pnlx/pnlx-pathmap.json`.

```sh
pnl list native
```

Example output:

```text
libnfc 1.8.0 /opt/homebrew/lib/libnfc.dylib
libusb-1.0 1.0.29 /opt/homebrew/lib/libusb-1.0.dylib
sdl2 2.32.10 /opt/homebrew/lib/libSDL2.dylib
```

### `pnl list repos`

Lists the repositories configured in `pnl.json`.

```sh
# First add a local file repository.
pnl repo add file file:///tmp/pnl-packages-demo

# Show the configured repositories.
pnl list repos
```

Example output:

```text
File file://packages
File file:///tmp/pnl-packages-demo
```

### `pnl repo add <git|file|https> <url> [--key <key>]`

Adds an extension-index repository to `pnl.json`. Duplicate URLs are ignored.

```sh
# Add a local index folder.
pnl repo add file file:///absolute/path/to/index

# Add a Git index repository.
pnl repo add git git@github.com:vendor/pnl-index.git

# Add an HTTPS index plus a public key for future signature checks.
pnl repo add https https://example.com/pnl/index.json --key ed25519:<public-key>
```

Result:

```jsonc
{
  // The repository kind.
  "type": "file",
  // The URL stored in pnl.json.
  "url": "file:///absolute/path/to/index"
}
```

Configured repositories are consulted by `pnl find` and by bare-name `pnl install`.

### `pnl repo index <dir> --base-url <url>`

Generates a `repository-index.json` for a directory of packages, so the repository can be browsed with `pnl find` without cloning. Each package directory containing a `pnlx.json` is recorded with its versions, the manifest path, a content `dist.sha256`, and an installable `source` URL of `<base-url>/<package-dir>`.

```sh
# Index the packages/ tree of a repository checkout.
pnl repo index packages \
  --base-url https://github.com/m3m0r7/pnl-packages/tree/main/packages
```

Options:

- `--output <file>` — where to write the index (default: `<dir>/repository-index.json`).
- `--reference <ref>` — the git reference recorded for every version (default: the package version).

Example output:

```text
indexed 106 package(s) into packages/repository-index.json
```

### `pnl repo remove <url>`

Removes a repository from `pnl.json`.

```sh
pnl repo remove file:///absolute/path/to/index
```

Nothing is printed on success.

### `pnl validate`

Checks that `pnl.json` — and, if present, `pnlx-lock.json` / `@pnlx/pnlx-pathmap.json` — is valid.

```sh
pnl validate
```

Example output:

```text
pnl workspace is valid
```

It checks:

- OpenAPI schema validation based on `schema_version`,
- package name and SemVer (version string) checks,
- environment-match checks for the lock/pathmap files,
- and pathmap/lock consistency checks.

### `pnl self-upgrade`

Upgrades `pnl` / `pnlx` themselves. It fetches the release tags from https://github.com/m3m0r7/pnl.git, and when a tag newer than the running version exists, it downloads that tag's source archive, builds it with `cargo build --release`, and installs it into a versioned layout:

```text
~/.local/share/pnl/versions/<version>/bin/pnl
~/.local/share/pnl/versions/<version>/bin/pnlx
~/.local/share/pnl/current -> versions/<version>
/usr/local/bin/pnl  -> ~/.local/share/pnl/current/bin/pnl
/usr/local/bin/pnlx -> ~/.local/share/pnl/current/bin/pnlx
```

Because `/usr/local/bin` holds only symlinks, an upgrade just swaps the `current` link — the binaries themselves are never overwritten in place.

```sh
pnl self-upgrade
# Place the symlinks somewhere other than /usr/local/bin.
pnl self-upgrade --bin-dir ~/bin
# Use a different install root.
pnl self-upgrade --home /opt/pnl
```

Example output when already up to date:

```text
pnl 0.1.5 is already the latest release in 0.25s
```

Notes:

- The install root follows the XDG Base Directory spec: `$XDG_DATA_HOME/pnl`, which defaults to `~/.local/share/pnl`. It can be changed with `--home` or the `PNL_HOME` environment variable (`--home` wins).
- Building requires a Rust toolchain (`cargo`).
- If `/usr/local/bin` is not writable, re-run with `sudo` or pass `--bin-dir`.
- Pre-release tags (e.g. `v1.0.0-rc.1`) are ignored.
- `self-upgrade` only manages the versioned symlink layout. If `pnl` was installed as a standalone binary (downloaded from the [releases page](https://github.com/m3m0r7/pnl/releases) and placed on `$PATH`), it cannot be swapped in place; `self-upgrade` will report the newer version and ask you to download it and reinstall.

### Update check on startup

`pnl` and `pnlx` check for a newer release when they start and print a one-line notice when one is available — `pnl self-upgrade` for a managed install, or "download and reinstall" for a standalone binary. The lookup is cached for one hour under `$XDG_CACHE_HOME/pnl`, so only the first run each hour goes over the network. The notice is shown only on an interactive terminal and can be turned off entirely by setting `PNL_NO_UPDATE_CHECK`.

### `pnl purge cache`

Removes the data `pnl` caches between runs — downloaded headers/libraries and the latest-release lookup — which all live under `$XDG_CACHE_HOME/pnl` (default `~/.cache/pnl`).

```sh
pnl purge cache
```


## pnlx Commands

`pnlx` is the tool for *authoring* library packages. If you're only using libraries, you may rarely touch anything beyond `pnlx build`.

### `pnlx help`

Prints the command summary and the available subcommands.

```sh
pnlx help
```

### `pnlx version`

Prints the `pnlx` version.

```sh
pnlx version
```

Example output:

```text
0.1.0
```

### `pnlx init`

Creates a default `pnlx.json` in an extension package folder if one doesn't exist.

```sh
# Create a folder for the new package.
mkdir -p packages/example

# Move into it.
cd packages/example

# Create pnlx.json if it doesn't exist.
pnlx init
```

Example output:

```text
initialized ./pnlx.json
```

### `pnlx validate`

Validates an extension package's `pnlx.json`.

```sh
# Clone the package repository for authoring.
git clone https://github.com/m3m0r7/pnl-packages.git

# Move into the libusb package folder.
cd pnl-packages/packages/libusb

# Validate pnlx.json and package-specific values.
pnlx validate
```

Example output:

```text
pnlx workspace is valid
```

### `pnlx gen <target> [--library-key <key>]`

Generates the PHP/Rust wrappers and friends under `src/generated` for an extension package.

```sh
# Clone the package repository for authoring.
git clone https://github.com/m3m0r7/pnl-packages.git

# Move into the libusb package folder.
cd pnl-packages/packages/libusb

# Generate the FFI definitions, classes, aliases, entrypoint, and bridge Rust.
pnlx gen libusb
```

Generated files:

```text
src/generated/libusb.ffi.php
src/generated/Libusb.php
src/generated/LibusbContext.php
src/generated/index.php
src/generated/function.aliases.php
src/generated/libusb.bridge.rs
```

This command:

- reads `pnlx.json`,
- resolves headers from `@pnlx/pnlx-pathmap.json` when run inside an installed project,
- falls back to the package's `headers` entries when the pathmap has none,
- and generates PHP classes, PHPDoc'd wrapper methods, aliases, FFI definitions, the entrypoint, and the Rust bridge.

When a single package needs multiple C libraries and the target name alone is ambiguous, use `--library-key`:

```sh
# Make the target's C library explicit.
pnlx gen libfoo --library-key libfoo-2.0
```

### `pnlx build [vendor/package ...]`

Rebuilds the compiled Rust bridges for installed packages.

```sh
# Build every installed bridge.
pnlx build

# Build by short name when it's unambiguous.
pnlx build libusb

# Build several at once.
pnlx build libusb libnfc libsdl

# Build by full vendor/package name.
pnlx build libusb/libusb
```

Example output:

```text
built 3 bridge(s)
```

This command:

- reads `pnlx-lock.json`,
- reads C library paths from `@pnlx/pnlx-pathmap.json`,
- compiles the installed `src/generated/*.bridge.rs` with `rustc --crate-type cdylib`,
- writes the resulting libraries under `@pnlx/packages/<vendor>/<package>/<version>/bridge/`,
- and updates the `bridges` entries in `@pnlx/pnlx-pathmap.json`.

### `pnlx package`

A reserved command. It currently returns a "not implemented" error.
