# Commands

[← Documentation index](../../README.md) · [日本語](../ja/commands.md)

## Table of Contents

- [pnl Commands](#pnl-commands)
  - [`pnl help`](#pnl-help)
  - [`pnl version`](#pnl-version)
  - [`pnl -i` / `pnl --information`](#pnl--i--pnl---information)
  - [`pnl -l` / `pnl --license`](#pnl--l--pnl---license)
  - [`pnl init`](#pnl-init)
  - [`pnl install <source>`](#pnl-install-source)
  - [`pnl config <key> [value]`](#pnl-config-key-value)
  - [`pnl compose <members...> --as <Class>`](#pnl-compose-members---as-class)
  - [`pnl update [vendor/package]`](#pnl-update-vendorpackage)
  - [`pnl uninstall <vendor/package>`](#pnl-uninstall-vendorpackage)
  - [`pnl list [glob]`](#pnl-list-glob)
  - [`pnl search [glob]`](#pnl-search-glob)
  - [`pnl info <package>`](#pnl-info-package)
  - [`pnl list native`](#pnl-list-native)
  - [`pnl list repos`](#pnl-list-repos)
  - [`pnl repo add <git|file|https> <url> [--key <key>]`](#pnl-repo-add-gitfilehttps-url---key-key)
  - [`pnl repo index <dir> --base-url <url>`](#pnl-repo-index-dir---base-url-url)
  - [`pnl repo sign <repository-index.json> --key <key>`](#pnl-repo-sign-repository-indexjson---key-key)
  - [`pnl repo remove <url>`](#pnl-repo-remove-url)
  - [`pnl validate`](#pnl-validate)
  - [`pnl doctor`](#pnl-doctor)
  - [`pnl self-upgrade`](#pnl-self-upgrade)
  - [Update check on startup](#update-check-on-startup)
  - [`pnl purge cache`](#pnl-purge-cache)
- [pnlx Commands](#pnlx-commands)
  - [`pnlx help`](#pnlx-help)
  - [`pnlx version`](#pnlx-version)
  - [`pnlx init`](#pnlx-init)
  - [`pnlx validate`](#pnlx-validate)
  - [`pnlx gen <target> [--library-key <key>]`](#pnlx-gen-target---library-key-key)
  - [`pnlx publish`](#pnlx-publish)
  - [`pnlx package`](#pnlx-package)

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
0.5.5
```

### `pnl -i` / `pnl --information`

Prints a neofetch-style banner: the pnl ASCII-art logo next to the version, OS and architecture, host, binary location, repository URLs, license, copyright, and the extensions installed in the current workspace with their install locations. `pnlx -i` / `pnlx --information` prints the same with the pnlx logo.

```sh
pnl -i
```

Example output:

```text
  ██████╗ ███╗   ██╗██╗        pnl 0.5.5
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
  "library_paths": [],
  // Global function generation is off initially.
  "features": {
    "global_functions": false
  },
  // No extensions installed yet.
  "extensions": {}
}
```

### `pnl install <source>`

Installs the given extension. Specifically, it finds the C library and headers, generates the PHP wrappers, and updates `pnl.json`, the lockfile, the pathmap, and `@pnlx/autoload.php`.

The source can be a URL, a local path, a **bare package name**, or a **distribution archive** (`.tar.gz`/`.tgz`/`.zip`, local or remote — it is downloaded if needed, extracted, and must contain a `pnlx.json`). You can pass **several sources at once** (`pnl install libusb libnfc`).

A bare name is resolved against your configured `repositories`, highest [`priority`](configuration.md#writing-pnljson) first, falling back to the built-in default repository `https://github.com/m3m0r7/pnl-packages` (kept internally at priority 0 — it is *not* written into `pnl.json`). pnl tries `repository-index.json` first; repositories with `key` require a sibling `repository-index.json.sig` Ed25519 signature, and index-selected packages are verified against `dist.sha256`. Append `@<version>` to pin a version — for git sources that checks out the matching tag/branch, and the resolved package version must match. Running `pnl install` with **no source** restores every extension from the lockfile at its locked version, re-verifying each package's content against the recorded sha256.

When the target package's `pnlx.json` declares `dependencies`, pnl resolves each dependency first at the newest version satisfying its version constraint. Already locked dependencies that satisfy the constraint are reused. The resolved dependency constraints are written into the lockfile.

If the package declares an `setup.install` recipe for your OS or Linux distro, `pnl install` offers to run it (e.g. `brew install …`) before resolving the native library, skipping it when the package's `check_if_exists` check already passes. Pass `-y` / `--yes` to accept that prompt automatically (or `-n` / `--no-interaction` to take the default). On Linux the recipe is selected from `/etc/os-release`: the distro `ID` (e.g. `alpine`, `ubuntu`, `fedora`) is tried first, then each `ID_LIKE` ancestor (e.g. `debian`, `rhel`), then a generic `linux` key. If the install commands fail, pnl reports which command failed and asks you to install the libraries and headers manually before running `pnl install` again.

Packages that declare `setup.install` or `setup.build_script` are checked against the `setup.build_script_hash` stamped into `pnlx.json` by `pnlx publish`. When the hash is missing or differs, interactive installs ask with a default of No. Under `-y`, pnl stops instead of trusting changed scripts. To override deliberately, pass `--allow-install-script-hash <sha256>` (repeatable). `--allow-unverified-install-scripts` is available as an explicit last resort. Packages installed from a built-in **authorized repository** (the first-party `m3m0r7/pnl-packages` registry; see the `repositories.authorized` whitelist baked into the binary) are trusted to run their install scripts and skip this prompt entirely.

Flags that adjust the generated PHP:

- `--alias-class <Class>` additionally exposes the extension under `<Class>` via `class_alias`, keeping the original class.
- `--function-prefix <prefix>` prepends `<prefix>` to every generated function and method name (the unprefixed names are not kept).
- `--enable-use-functions` persists `features.global_functions = true` into `pnl.json`, exposing the generated global-functions API (see [Configuration](configuration.md#writing-pnljson)).
- `--enable-allow-cdata` persists `features.cdata_arguments = true`, so generated signatures also accept a raw `\FFI\CData`.
- `--enable-use-php-scalars-in-return` persists `features.scalar_returns = true`, so methods return native `int`/`float`/`string` for scalars that fit.
- `--enable-use-php-scalars-in-const` persists `features.scalar_constants = true`, so `const.php` uses native scalars instead of `\Pnlx\Types\*` wrappers where lossless.
- `--enable-static-inline` persists `compile_options.static_inline = true`, so a library's `static inline` functions are compiled into a callable shim instead of throwing stubs (needs a C compiler — see [Configuration](configuration.md#static-inline-functions-compile_options)).

Flags that gate install scripts and integrity:

- `--allow-install-script-hash <sha256>` trusts the given install-script hash for this run. It can be repeated.
- `--allow-unverified-install-scripts` allows missing or changed install-script hashes.
- `-f` / `--force` reinstalls even when the resolved content no longer matches the sha256 recorded in the lockfile; instead of aborting, it warns and overwrites the locked digest with the new content. When run interactively without the flag, a digest mismatch prompts before overwriting (default: no).

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
  ✓ resolved libusb-1.0 1.0.29 libusb-1.0.dylib
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/libusb.ffi.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/Libusb.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbContext.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbException.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbLibraryComponent.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbManifest.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/const.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/index.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/function.aliases.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/macro.functions.php
  ✓ installed extension libusb/libusb

added 1 extension in 1.42s
```

The main files produced:

```text
@pnlx/packages/libusb/libusb/1.0.29/      ← the installed package + its src/generated
pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
@pnlx/ide-helper.php                       ← IDE/static-analysis stub for the entities
```

See [`pnlx gen`](#pnlx-gen-target---library-key-key) for the full set of files written under `src/generated`.

#### Content integrity

When a package is installed, `pnl` computes a content signature for it: every file is hashed with sha256 (in sorted order), and those per-file hashes are hashed together into one digest, which is recorded as `dist.sha256` in `pnlx-lock.json`. On a later install of the **same version**, the freshly downloaded content is hashed again and compared to the locked digest; if they differ, the install is aborted with an integrity error because the content was modified or tampered with. Installing a new version is treated as a legitimate update. (Generated output, `.git`, and the workspace directory are excluded from the digest.)

### `pnl config <key> [value]`

Gets or sets a `pnl.json` configuration value, git-config style. With no value it prints the current one; with a value it sets the key; `--unset` resets the key to its default.

```sh
pnl config compile_options.static_inline          # print the current value
pnl config compile_options.static_inline true     # set it (true/1/yes/on or false/0/no/off)
pnl config compile_options.static_inline --unset   # reset to the default
```

The value is validated against the typed schema, so an unknown key or a non-boolean value for a boolean key is rejected. Known keys are `compile_options.static_inline`, the `features.*` switches, and `output_dir`.

Because these settings change generated output, after a successful change `pnl config` offers to reinstall so it takes effect (interactive only — answer no to apply it later with `pnl install`; a non-interactive run just prints a reminder).

### `pnl compose <members...> --as <Class>`

Composes two or more **installed** extensions into a single class that exposes all their functions through one shared FFI scope. This is the named, file-backed counterpart to `Pnlx\Runtime::compose([...])` (see [PHP Usage](php-usage.md#runtimecompose-share-one-ffi-scope-across-packages)) — generating it as a real file gives editors and static analysis something to see, and the composed methods are real (not a `__call` proxy), so by-reference out-parameters round-trip.

Because the members share one scope, a `CData` produced by one package (e.g. an `SDL2_image` surface from `Sdlimage::IMG_Load`) flows straight into another (`Libsdl::SDL_CreateTextureFromSurface`).

```sh
# Fuse the SDL and SDL_image extensions into one Pnlx\Sdlx\Sdlx class.
pnl compose libsdl sdlimage --as 'Pnlx\Sdlx\Sdlx'
```

Arguments and options:

- `<members...>` — two or more installed package names (`vendor/package` or the bare leaf).
- `--as <Class>` — the fully-qualified class name to generate (required).
- `--prefix <prefix>` — method-name prefix used to resolve trait-method collisions when two members expose a same-named function (reserved).

It writes the composite under `@pnlx/composites/<Class>.php` (and `<Class>Functions.php` when `features.global_functions` is enabled), records the composite in `pnl.json`, and regenerates `@pnlx/autoload.php` so the new class is loaded after its members.

```text
composed ./@pnlx/composites/Sdlx.php
composed Pnlx\Sdlx\Sdlx from libsdl, sdlimage
```

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
- and re-runs install, generation, and pathmap updates.

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

### `pnl search [glob]`

> Aliased as `pnl find` — both invoke the same command.

Lists the packages **available** from your configured `repositories` plus the built-in default repository, optionally filtered by a glob pattern. Like `pnl list`, the pattern matches the full `vendor/extension` name or its leaf.

Each repository is enumerated cheaply when it publishes a `repository-index.json` (fetched over HTTP for GitHub/`https` repositories, or read from disk for `file` ones); otherwise pnl falls back to a shallow clone and a directory walk. When two repositories offer the same package, the higher-[`priority`](configuration.md#writing-pnljson) one wins (the default repository is consulted last).

```sh
# Browse everything the default repository offers.
pnl search

# Only packages whose name starts with "lib".
pnl search 'lib*'
```

Example output (name, available versions, source repository):

```text
libusb/libusb 1.0.29 https://github.com/m3m0r7/pnl-packages/tree/main/packages
libuv/libuv 1.48.0 https://github.com/m3m0r7/pnl-packages/tree/main/packages
```

### `pnl info <package>`

Shows a package's remote details — its install commands, the headers it reads, and the native libraries it links — fetched from the repository even when the package is already installed locally. The target can be a bare name, `vendor/package`, a URL, or a path.

```sh
# Describe the libusb package without installing it.
pnl info libusb
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
# Add a local index folder by file:// URL.
pnl repo add file file:///absolute/path/to/index

# Add a local index folder by plain path (absolute or relative to the project).
pnl repo add file /Users/me/work/pnl-packages
pnl repo add file ./vendor-packages

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

Configured repositories are consulted by `pnl search` and by bare-name `pnl install`. A `file` repository may point at any **local directory** — pass it as a `file://` URL or as a plain filesystem path (absolute, or relative to the project root). pnl reads such a repository straight from disk, preferring a committed `repository-index.json` and otherwise walking the tree for package folders.

### `pnl repo index <dir> --base-url <url>`

Generates a `repository-index.json` for a directory of packages, so the repository can be browsed with `pnl search` without cloning. Each package directory containing a `pnlx.json` is recorded with its versions, the manifest path, a content `dist.sha256`, and an installable `source` URL of `<base-url>/<package-dir>`.

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

### `pnl repo sign <repository-index.json> --key <key>`

Writes a detached signature for `repository-index.json`. By default the signature is written beside the index as `repository-index.json.sig` in `ed25519:<base64>` form. The secret key is a 32-byte Ed25519 seed as `ed25519:<base64>` or 64 hex characters. The command prints the matching public `repository key`; pass that to `pnl repo add ... --key <key>` to make install/search verify the index signature.

```sh
pnl repo sign packages/repository-index.json --key ed25519:<base64-secret>
pnl repo add https https://example.com/pnl/packages --key ed25519:<base64-public>
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

### `pnl doctor`

Diagnoses the local environment for installing and running pnl extensions.

```sh
pnl doctor
```

It checks:

- **libclang** — required for binding generation (`pnl install`); a failure here is the one fatal check.
- **C compiler** — optional, only needed for `compile_options.static_inline` shims.
- **pkg-config** — informational: pnl parses `.pc` files itself, so the system pkg-config is not required.
- **PHP + FFI** — that `php` is on `PATH` and the FFI extension is loaded (with the `ffi.enable` setting noted).
- **workspace** — whether a `pnl.json` is present and how many extensions are locked.

It exits non-zero if any required check fails.

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
pnl 0.5.5 is already the latest release in 0.25s
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

`pnlx` is the tool for *authoring* library packages. If you're only using libraries, you may rarely need it directly.

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
0.5.5
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

Generates the PHP FFI definitions, wrappers, and friends under `src/generated` for an extension package.

```sh
# Clone the package repository for authoring.
git clone https://github.com/m3m0r7/pnl-packages.git

# Move into the libusb package folder.
cd pnl-packages/packages/libusb

# Generate the FFI definitions, classes, aliases, and entrypoint.
pnlx gen libusb
```

Generated files (`Libusb` stands in for the package's class name):

```text
src/generated/libusb.ffi.php              # the FFI cdef wrapped as PHP
src/generated/Libusb.php                  # the entity class (static wrapper methods)
src/generated/LibusbContext.php           # the CData handle wrapper
src/generated/LibusbException.php         # the package's generated exception
src/generated/LibusbLibraryComponent.php  # trait used by Runtime::compose / pnl compose
src/generated/LibusbManifest.php          # install-time metadata (name, version, path)
src/generated/const.php                   # generated constants
src/generated/index.php                   # the entrypoint that boots the extension
src/generated/function.aliases.php        # function-name aliases
src/generated/functions.php               # \Pnlx\Func global functions (global_functions)
src/generated/macro.functions.php         # function-like macros surfaced as functions
src/generated/types/                      # one file per struct/typedef wrapper
src/generated/enums/                      # one PHP enum per named C enum
src/generated/symbol/                     # marker classes for exported C data symbols
```

Feature-dependent variants are emitted alongside these: `cdata/` (when `cdata_arguments`) and `scalar/` (when `scalar_returns`/`scalar_constants`).

This command:

- reads `pnlx.json`,
- resolves headers from `@pnlx/pnlx-pathmap.json` when run inside an installed project,
- falls back to the package's `headers` entries when the pathmap has none,
- and generates PHP classes, PHPDoc'd wrapper methods, aliases, FFI definitions, constants, enums, type wrappers, and the entrypoint.

When a single package needs multiple C libraries and the target name alone is ambiguous, use `--library-key`:

```sh
# Make the target's C library explicit.
pnlx gen libfoo --library-key libfoo-2.0
```

### `pnlx publish`

Updates publish-time metadata in `pnlx.json`. Currently it hashes every `setup.install` command, or the package-relative script contents referenced by `setup.build_script`, and writes the resulting sha256 into `setup.build_script_hash`.

```sh
pnlx publish
```

`setup.build_script` is mutually exclusive with `setup.install`. The script path must stay inside the package: absolute paths and `..` traversal are rejected.

### `pnlx package`

A reserved command. It currently returns a "not implemented" error.
