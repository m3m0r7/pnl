# pnl

[日本語版](README.ja.md)

## Table Of Contents

- [What Is pnl?](#what-is-pnl)
- [The Big Picture](#the-big-picture)
- [Quick Start](#quick-start)
- [Status](#status)
- [Requirements](#requirements)
- [Build And Install](#build-and-install)
- [Project Layout](#project-layout)
- [Writing `pnl.json`](#writing-pnljson)
- [Install Sources](#install-sources)
- [pnl Commands](#pnl-commands)
- [pnlx Commands](#pnlx-commands)
- [PHP Usage](#php-usage)
- [Generated Files](#generated-files)
- [Validation And Development](#validation-and-development)
- [Schemas](#schemas)
- [Limitations](#limitations)
- [License](#license)

## What Is pnl?

In one sentence: **pnl makes it easy to use C libraries from PHP.**

There are many battle-tested libraries out there written in C — `libusb` for talking to USB devices, `libnfc` for NFC, `SDL` for windows, graphics, and sound. The catch is that almost all of them are built for C, and calling them from PHP by hand is painful.

PHP does have FFI (Foreign Function Interface), a mechanism for calling other languages' libraries directly. But using it yourself means copying function signatures by hand, juggling raw pointers, and a lot of fiddly bookkeeping.

pnl automates that pain away. Concretely, it:

- installs library "packages",
- finds the C library and its headers (type information) already installed on your machine,
- **generates wrappers** (PHP classes and methods) so you can call the library from PHP,
- automatically compiles a small **bridge** (glue code written in Rust) that connects PHP and C,
- and exposes everything through the `Pnlx` PHP SDK.

Think of it like Composer, but for using C libraries from PHP.

This repository ships two command-line tools:

- **`pnl`**: the tool for *using* libraries — install, lockfile management, validation, and listing.
- **`pnlx`**: the tool for *authoring* library packages — generating wrappers and rebuilding installed bridges.

If you just want to use libraries, `pnl` is usually all you need.

One thing to note: the generated install state lives in a dedicated `@pnlx/` directory, not Composer's `vendor/`. In your PHP code you load Composer's autoload first (for the SDK), then `@pnlx/autoload.php` (for the installed extensions).

## The Big Picture

A typical first-time flow looks like this:

1. Build/install `pnl` and `pnlx` ([Build And Install](#build-and-install)).
2. Run `pnl init` in your project to create the `pnl.json` config file.
3. Run `pnl install <library URL>` to add the library you want.
4. Call the library from PHP through `Pnlx\Runtime` ([PHP Usage](#php-usage)).

If you just want to get a feel for it, start with the install in step 3 and the sample code in step 4.

## Quick Start

The smallest possible example: call C's `printf` from PHP. The C standard library (`libc`) ships with every operating system — macOS, Linux, and Windows — so there is **nothing to install** beyond pnl itself.

In your project directory:

```sh
# 1. Create pnl.json (it already lists the official package repository).
pnl init

# 2. Add the libc package. A bare name is resolved against the repository.
pnl install libc
```

Then call it from PHP (`quickstart.php`):

```php
<?php

declare(strict_types=1);

require_once __DIR__ . '/vendor/autoload.php';   // the Pnlx SDK (via Composer)
require_once __DIR__ . '/@pnlx/autoload.php';    // the installed extensions

use Pnlx\Libc\Libc;
use Pnlx\Runtime;

$runtime = new Runtime(__DIR__);

/** @var Libc $libc */
$libc = $runtime->load(Libc::class);

$libc->printf("Hello, World from libc!\n");
$libc->puts("And this line is printed by libc puts.");
```

```sh
php quickstart.php
```

```text
Hello, World from libc!
And this line is printed by libc puts.
```

Why `libc` is a good first package: its functions (`printf`, `puts`) come from the C runtime that is already part of the OS, so no `brew install` / `apt-get install` step is needed. Its library entries are declared `"virtual": true` in the package, which tells pnl to link them by name without expecting a file on disk (on macOS, libc lives only in the dyld shared cache). When you are ready for a real library, try [`pnl install libusb`](#pnl-install-source).

## Status

This repository is an early implementation (a prototype).

Installing directly from a local path, a `file://` URL, or a Git URL (GitHub, GitLab, Bitbucket, or any generic host) works for any package that contains `pnlx.json`. A package's native libraries and headers can also be fetched over `http(s)`, `ftp`, or `git` (see [Native library discovery](#native-library-discovery)). On the other hand, repository-index dependency solving and signed package indexes are still design-stage work.

## Requirements

At a minimum, you need the following:

| Tool | Version | What it's for |
| --- | --- | --- |
| Rust | 1.85+ recommended | Uses Rust 2024 edition. `rustc` also compiles the bridge libraries. |
| PHP CLI | 8.2+ recommended | The `ffi` extension must be loaded and `ffi.enable` must allow CLI FFI. |
| Composer | 2.x | Installs PHPUnit, PHPStan, php-cs-fixer, and `cebe/php-openapi`. |
| Git | 2.x | Needed for Git install sources. |
| Make | any POSIX make | Used by the included `Makefile`. |
| pkg-config | optional but recommended | Used to discover C library versions and include paths. |
| C libraries | per package | The libusb/libnfc/SDL examples need the matching libraries and headers. |

The environment currently validated locally:

```text
rustc 1.92.0
cargo 1.92.0
PHP 8.5.2 CLI NTS
Composer 2.8.8
git 2.52.0
pkg-config 2.5.1
macOS/aarch64 with Homebrew libusb, libnfc, SDL2
```

Whether PHP can use FFI is checked at runtime. If PHP cannot use FFI, the SDK raises an FFI-related exception before loading any bridge.

## Build And Install

Build the release binaries:

```sh
# Build both pnl and pnlx in release mode.
make build
```

On success:

```text
cargo build --release --bins
target/release/pnl
target/release/pnlx
```

Install the binaries to a prefix of your choice:

```sh
# Copy the release binaries into /usr/local/bin.
make install PREFIX=/usr/local
```

On success:

```text
install -d "/usr/local/bin"
install -m 0755 target/release/pnl "/usr/local/bin/pnl"
install -m 0755 target/release/pnlx "/usr/local/bin/pnlx"
```

During development you can also call `target/debug/pnl` and `target/debug/pnlx` directly after `cargo build`. That said, the examples in this README assume the installed `pnl` / `pnlx` binaries.

GitHub Actions builds release archives for Linux, macOS, and Windows. Every workflow run uploads artifacts, and tag pushes such as `v0.1.0` attach those archives to a GitHub Release.

Example release archive names:

```text
pnl-<version>-x86_64-unknown-linux-gnu.tar.gz
pnl-<version>-x86_64-apple-darwin.tar.gz
pnl-<version>-aarch64-apple-darwin.tar.gz
pnl-<version>-x86_64-pc-windows-msvc.zip
```

## Project Layout

After you install extensions, a PHP project looks like this. Everything under `@pnlx/` is generated, so you normally don't edit it by hand:

```text
project-root/
  composer.json
  pnl.json                     ← the config file you edit
  @pnlx/                       ← everything below is generated
    autoload.php
    pnlx-lock.json
    pnlx-pathmap.json
    packages/
      vendor/
        package/
          <version>/            ← one directory per installed version
            pnlx.json
            src/generated/
            bridge/             ← the compiled native bridge for this version
              <package>.bridge.rs
              lib<package>_bridge.dylib
```

Files worth remembering:

- `pnl.json`: the project config file you edit.
- `@pnlx/pnlx-lock.json`: the generated lockfile for your current environment (a record that pins installed versions).
- `@pnlx/pnlx-pathmap.json`: a generated "map" of where the C libraries, headers, and bridges live for your current environment.
- `@pnlx/autoload.php`: the generated PHP entrypoint that loads every installed package at once.

## Writing `pnl.json`

`pnl.json` is your project's config file. It describes:

- where extension packages may be discovered from,
- which folders to search for the C libraries,
- whether optional features are enabled,
- and which extensions you want installed.

Minimal config:

```jsonc
{
  // Schema version used for validation (selects schemas/pnl/<version>/schema.json).
  "schema_version": "2026-07-01",
  // If you only install directly from a URL, you can leave repositories empty.
  "repositories": [],
  // Extra folders to search for C libraries.
  "load_paths": [],
  // Switches for optional features.
  "features": {
    // Keep generated global functions off by default.
    "use_functions": false
  },
  // The extensions you want, listed as vendor/package.
  "extensions": {}
}
```

A typical local-development config:

```jsonc
{
  // Schema version for this pnl.json.
  "schema_version": "2026-07-01",
  // Where packages may be fetched from (optional).
  "repositories": [
    {
      // A local file repository.
      "type": "file",
      "url": "file://packages"
    }
  ],
  // C library folders to check before system defaults.
  "load_paths": [
    "/opt/homebrew/lib",
    "/usr/local/lib"
  ],
  // Generate C-style global PHP functions so you can use them.
  "features": {
    "use_functions": true
  },
  // The extensions this project needs.
  "extensions": {
    "libusb/libusb": {
      // Accept libusb wrappers in the 1.x line.
      "version": ">=1.0.0 & <2.0.0",
      // Treat this extension as required for the project.
      "required": true
    }
  }
}
```

What each field means:

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `schema_version` | string | yes | The config schema version. Currently `2026-07-01`. |
| `repositories` | array | yes | Where packages come from. Currently used as a file-repository fallback; full index solving is not complete. |
| `load_paths` | array | yes | Folders for C libraries, checked before system defaults and environment-derived paths. |
| `output_dir` | string | no | Directory (relative to the project root) for generated workspace files — the lock, pathmap, installed packages, and autoload. Defaults to `@pnlx`. |
| `features.use_functions` | boolean | yes | When `true`, generated entrypoints define PHP functions named after the C functions under the `\Pnlx\Func` namespace. |
| `extensions` | object | yes | The extensions you want, keyed by `vendor/package`. `pnl install` adds entries here automatically. |

Examples of `repositories` entries:

```jsonc
// A local package index.
{ "type": "file", "url": "file://packages" }

// A Git-backed package index.
{ "type": "git", "url": "git@github.com:vendor/pnl-index.git" }

// An HTTPS index with a reserved signing-key field.
{ "type": "https", "url": "https://example.com/pnl/index.json", "key": "ed25519:<public-key>" }
```

`type` can be `file`, `git`, or `https`. `key` is optional and reserved for future signed indexes. If you pass a local path, a `file://` URL, or a Git URL directly to `pnl install`, you don't need a `repositories` entry at all.

`load_paths` are folders for the C *library* files (`.so` / `.dylib`, etc.), not header (include) folders. Header lookup uses `pkg-config`, C include environment variables, package-local includes, and common system include directories.

### Native library discovery

By default `pnl install` looks for each required C library on the local machine (`load_paths`, `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`, `PATH`, and common system folders) and for headers via `pkg-config`/include paths. A package's `pnlx.json` can instead point a requirement at a remote source, in which case the asset is downloaded once and cached:

```jsonc
"requires": {
  "mylib-1.0": {
    "library_names": ["libmylib.so", "mylib.dll"],
    // Fetch the binary over http(s), ftp, or a git tree URL instead of $PATH.
    "library_url": "https://example.com/releases/libmylib.so",
    // Fetch the header, or...
    "header_url": "https://raw.githubusercontent.com/acme/mylib/v1.0/mylib.h",
    // ...embed it inline when no file is available.
    "header_inline": "int mylib_add(int a, int b);\nconst char *mylib_version(void);\n",
    "symbol_prefix": "mylib_",
    "version": "^1.0.0",
    "required": true
  }
}
```

Supported source schemes are `http`/`https`, `ftp`, and `git` (use a `/tree/<branch>/<path>` URL or an `ssh`/`git`/`.git` URL to fetch a file from a repository). `ftps` is not implemented yet — use `https` or `ftp`. Local discovery remains the fallback whenever a requirement has no remote source.

Example of pinning an extension's version:

```jsonc
// Extension entries go under the top-level "extensions" object.
"extensions": {
  // The key is in vendor/package form.
  "vendor/package": {
    // Exact, range, caret (^), and tilde (~) constraints are supported.
    "version": "^1.2.3",
    // Treat this as a required extension.
    "required": true
  }
}
```

Version constraints support exact versions, comparison ranges, caret and tilde. Combine comparators with `&` (and) and `|` (or), where `&` binds tighter than `|`, and group with parentheses — e.g. `>=1.0.0 & <2.0.0`, or `>=1.0.0 & <2.0.0 | >=3.0.0`, or `(>=1.0.0 & <2.0.0) | >=3.0.0`. A bare version such as `1.2.3` means an exact match. `required` is currently just a note about dependency intent; in this MVP, install still expects you to name the source explicitly.

Example of global-function mode:

```jsonc
// Optional features go under the top-level "features" object.
"features": {
  // true lets generated entrypoints define C-style global PHP functions.
  "use_functions": true
}
```

When enabled, a generated package entrypoint defines namespaced functions under `\Pnlx\Func\<Class>` (one segment per package), such as `\Pnlx\Func\Libusb\libusb_init()` — but only when no function of that name already exists. Call them fully qualified, or import them with `use function Pnlx\Func\Libusb\libusb_init;` and then call `libusb_init()`. Keeping them under a namespace avoids clobbering the global namespace. When disabled, you call methods on the object you got from `$runtime->load(...)` instead.

## Install Sources

`pnl install` accepts more than just `packages/<name>` local paths — you can specify the source in several forms.

Currently supported:

```sh
# Install from a local extension folder.
pnl install /absolute/path/to/extension-root

# Install from a file:// URL.
pnl install file:///absolute/path/to/extension-root

# Install from a package folder inside a GitHub repository.
pnl install https://github.com/m3m0r7/pnl-packages/packages/libusb

# Install from a GitHub tree URL ("main" becomes the branch to clone).
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# Install from an scp-style SSH URL with a package subfolder.
pnl install git@github.com:m3m0r7/pnl-packages/packages/libusb
```

For GitHub HTTPS URLs and scp-style SSH URLs, the first two path segments after the host (`owner/repository`) are treated as the repository, and the rest is treated as the package's location inside it. GitHub URLs using `/tree/<branch>/...` are also accepted; there, `<branch>` is the branch to clone and the rest is the package location.

For example, these URLs:

```text
https://github.com/m3m0r7/pnl-packages/packages/libusb
https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
git@github.com:m3m0r7/pnl-packages/packages/libusb
```

are cloned as:

```text
https://github.com/m3m0r7/pnl-packages.git
git@github.com:m3m0r7/pnl-packages.git
```

and installed from:

```text
packages/libusb
```

The clone is placed temporarily in the system temp directory — somewhere like `/tmp/pnl/git/...` on Linux, or `/var/folders/.../T/pnl/git/...` on macOS. Only the resolved package folder that contains `pnlx.json` is copied into `@pnlx/packages/<vendor>/<package>/<version>`.

In every case, install fails if the resolved local path does not contain `pnlx.json`.

FTP/FTPS sources are recognized, but they fail with a clear error until a downloader and signature verification are implemented.

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

The source can be a URL, a local path, or a **bare package name** that is resolved against the configured `repositories` (the default `pnl.json` ships the official `pnl-packages` repository). Append `@<version>` to pin a specific version — for git sources that checks out the matching tag/branch, and the resolved package version must match. Running `pnl install` with **no source** restores every extension from the lockfile at its locked version, re-verifying each package's content against the recorded sha256.

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
@pnlx/pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
```

#### Content integrity

When a package is installed, `pnl` computes a content signature for it: every file is hashed with sha256 (in sorted order), and those per-file hashes are hashed together into one digest, which is recorded as `dist.sha256` in `@pnlx/pnlx-lock.json`. On a later install of the **same version**, the freshly downloaded content is hashed again and compared to the locked digest; if they differ, the install is aborted with an integrity error because the content was modified or tampered with. Installing a new version is treated as a legitimate update. (Generated output, `.git`, and the workspace directory are excluded from the digest.)

### `pnl update [vendor/package]`

Reinstalls one package — or every installed package — from the source recorded in the lockfile.

```sh
# Reinstall just libusb from its recorded source.
pnl update libusb/libusb

# Reinstall every installed extension.
pnl update
```

This command:

- reads `@pnlx/pnlx-lock.json`,
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

### `pnl list`

Lists installed extensions. `pnl list` is the same as `pnl list extensions`.

```sh
pnl list
pnl list extensions
```

Example output:

```text
libnfc/libnfc 1.8.0 1.8.0
libsdl/libsdl 2.32.10 2.32.10
libusb/libusb 1.0.29 1.0.29
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

Note that repository-index resolution is not complete yet. This command records configuration for the planned resolver and for fallback discovery of local file repositories.

### `pnl repo remove <url>`

Removes a repository from `pnl.json`.

```sh
pnl repo remove file:///absolute/path/to/index
```

Nothing is printed on success.

### `pnl validate`

Checks that `pnl.json` — and, if present, `@pnlx/pnlx-lock.json` / `@pnlx/pnlx-pathmap.json` — is valid.

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

A reserved command. It currently returns a "not implemented" error.

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

- reads `@pnlx/pnlx-lock.json`,
- reads C library paths from `@pnlx/pnlx-pathmap.json`,
- compiles the installed `src/generated/*.bridge.rs` with `rustc --crate-type cdylib`,
- writes the resulting libraries under `@pnlx/packages/<vendor>/<package>/<version>/bridge/`,
- and updates the `bridges` entries in `@pnlx/pnlx-pathmap.json`.

### `pnlx package`

A reserved command. It currently returns a "not implemented" error.

## PHP Usage

First, install the sample packages:

```sh
# Install the libusb, libnfc, and SDL wrappers.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libnfc
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libsdl

# Rebuild the bridges after installing.
pnlx build
```

### libusb: version, error name, and device count

An example that prints libusb's version, an error name, and how many devices are connected:

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libusb\Libusb;
use Pnlx\Runtime;

// Runtime resolves the config, pathmap, generated entrypoints, and bridge FFI.
$runtime = new Runtime(__DIR__);

/** @var Libusb $libusb */
// Get the generated libusb object through Runtime.
$libusb = $runtime->load(Libusb::class);

// Read package metadata and the compiled bridge path.
$context = $runtime->context(Libusb::class);

printf("extension: %s %s\n", $context->name(), $context->version());
printf("bridge: %s\n", $context->path());
printf("error name for 0: %s\n", $libusb->libusb_error_name(0));
printf("strerror for 0: %s\n", $libusb->libusbStrerror(0));

// Initialize libusb with the default context.
$result = $libusb->libusbInit(null);
printf("libusb_init: %d (%s)\n", $result, $libusb->libusbErrorName($result));

if ($result === 0) {
    // Allocate void *[1] without exposing raw FFI::new() to user code.
    $deviceList = $runtime->allocator()->voidPointerArray(1);

    // libusb writes the device-list pointer into $deviceList[0].
    $deviceCount = $libusb->libusbGetDeviceList(null, $deviceList);

    if ($deviceCount < 0) {
        // Negative values are libusb error codes.
        printf("device count: failed (%s)\n", $libusb->libusbErrorName($deviceCount));
    } else {
        printf("device count: %d\n", $deviceCount);

        // Release the device list returned by libusb_get_device_list().
        $libusb->libusbFreeDeviceList($deviceList[0], 1);
    }

    // Shut down the default libusb context.
    $libusb->libusbExit(null);
    echo "libusb_exit: ok\n";
}
```

Example output:

```text
extension: libusb/libusb 1.0.29
bridge: /path/to/project/@pnlx/packages/libusb/libusb/1.0.27/bridge/libusb_bridge.dylib
error name for 0: LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED
strerror for 0: Success
libusb_init: 0 (LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED)
device count: 6
libusb_exit: ok
```

### SDL: open a window (object methods)

Opening an SDL window and drawing "Hello World!" inside it using the generated object's methods:

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use Pnlx\Runtime;
use function Pnlx\Util\is_null;

// Flag for SDL's video subsystem.
const SDL_INIT_VIDEO = 0x00000020;

// Value that asks SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Flag for creating a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

// Runtime loads the generated SDL object and its bridge.
$runtime = new Runtime(__DIR__);

/** @var Libsdl $sdl */
// Use methods like SDL_Init() and SDL_CreateWindow().
$sdl = $runtime->load(Libsdl::class);

// A tiny 5x7 bitmap font for the characters in "Hello World!".
// '1' marks a lit pixel; rows are top-to-bottom.
$font = [
    'H' => ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'e' => ['00000', '00000', '01110', '10001', '11111', '10000', '01110'],
    'l' => ['01100', '00100', '00100', '00100', '00100', '00100', '01110'],
    'o' => ['00000', '00000', '01110', '10001', '10001', '10001', '01110'],
    'W' => ['10001', '10001', '10001', '10101', '10101', '11011', '10001'],
    'r' => ['00000', '00000', '10110', '11001', '10000', '10000', '10000'],
    'd' => ['00001', '00001', '01101', '10011', '10001', '10001', '01111'],
    '!' => ['00100', '00100', '00100', '00100', '00100', '00000', '00100'],
    ' ' => ['00000', '00000', '00000', '00000', '00000', '00000', '00000'],
];

// Declare handles outside try so cleanup can see them.
$window = null;
$renderer = null;
$initialized = false;

try {
    // Start SDL's video subsystem.
    $result = $sdl->SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . $sdl->SDL_GetError());
    }
    $initialized = true;

    // Create a window and a renderer to draw into it.
    $window = $sdl->SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . $sdl->SDL_GetError());
    }

    $renderer = $sdl->SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . $sdl->SDL_GetError());
    }

    // Clear to a dark background.
    $sdl->SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    $sdl->SDL_RenderClear($renderer);

    // Draw "Hello World!" in the window, scaling each font pixel into a block.
    // SDL_RenderDrawPoint takes only integers, so no FFI structs are needed.
    $sdl->SDL_SetRenderDrawColor($renderer, 0xFF, 0xFF, 0xFF, 0xFF);
    $scale = 6;
    $x = 70;
    $y = 150;
    foreach (str_split('Hello World!') as $char) {
        $glyph = $font[$char] ?? $font[' '];
        foreach ($glyph as $row => $bits) {
            for ($col = 0; $col < 5; $col++) {
                if ($bits[$col] !== '1') {
                    continue;
                }
                for ($dy = 0; $dy < $scale; $dy++) {
                    for ($dx = 0; $dx < $scale; $dx++) {
                        $sdl->SDL_RenderDrawPoint($renderer, $x + $col * $scale + $dx, $y + $row * $scale + $dy);
                    }
                }
            }
        }
        $x += 6 * $scale; // 5px glyph + 1px gap
    }

    // Present the frame and keep the window up briefly while pumping events.
    $sdl->SDL_RenderPresent($renderer);
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        $sdl->SDL_PumpEvents();
        $sdl->SDL_Delay(16);
    }
} finally {
    if (!is_null($renderer)) {
        $sdl->SDL_DestroyRenderer($renderer);
    }
    // Destroy the window if creation succeeded.
    if (!is_null($window)) {
        $sdl->SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        $sdl->SDL_Quit();
    }
}
```

### SDL: open a window (global functions)

Opening an SDL window and drawing "Hello World!" inside it using the generated global functions. To use this style, first set `features.use_functions` to `true` in `pnl.json`.

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use function Pnlx\Util\is_null;

// Generated global functions live under \Pnlx\Func; import the ones used here
// so the short names below resolve.
use function Pnlx\Func\Libsdl\{
    SDL_Init,
    SDL_GetError,
    SDL_CreateWindow,
    SDL_CreateRenderer,
    SDL_SetRenderDrawColor,
    SDL_RenderClear,
    SDL_RenderDrawPoint,
    SDL_RenderPresent,
    SDL_PumpEvents,
    SDL_Delay,
    SDL_DestroyRenderer,
    SDL_DestroyWindow,
    SDL_Quit,
};

// Flag for SDL's video subsystem.
const SDL_INIT_VIDEO = 0x00000020;

// Value that asks SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Flag for creating a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

if (!function_exists('Pnlx\\Func\\Libsdl\\SDL_Init')) {
    // @pnlx/autoload.php defines \Pnlx\Func functions only when features.use_functions is true.
    throw new RuntimeException('SDL global functions are disabled. Set pnl.json features.use_functions to true.');
}

// A tiny 5x7 bitmap font for the characters in "Hello World!".
// '1' marks a lit pixel; rows are top-to-bottom.
$font = [
    'H' => ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'e' => ['00000', '00000', '01110', '10001', '11111', '10000', '01110'],
    'l' => ['01100', '00100', '00100', '00100', '00100', '00100', '01110'],
    'o' => ['00000', '00000', '01110', '10001', '10001', '10001', '01110'],
    'W' => ['10001', '10001', '10001', '10101', '10101', '11011', '10001'],
    'r' => ['00000', '00000', '10110', '11001', '10000', '10000', '10000'],
    'd' => ['00001', '00001', '01101', '10011', '10001', '10001', '01111'],
    '!' => ['00100', '00100', '00100', '00100', '00100', '00000', '00100'],
    ' ' => ['00000', '00000', '00000', '00000', '00000', '00000', '00000'],
];

// Declare handles outside try so cleanup can see them.
$window = null;
$renderer = null;
$initialized = false;

try {
    // Start SDL's video subsystem through the global function.
    $result = SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . SDL_GetError());
    }
    $initialized = true;

    // Create a window and a renderer to draw into it.
    $window = SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . SDL_GetError());
    }

    $renderer = SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . SDL_GetError());
    }

    // Clear to a dark background.
    SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    SDL_RenderClear($renderer);

    // Draw "Hello World!" in the window, scaling each font pixel into a block.
    // SDL_RenderDrawPoint takes only integers, so no FFI structs are needed.
    SDL_SetRenderDrawColor($renderer, 0xFF, 0xFF, 0xFF, 0xFF);
    $scale = 6;
    $x = 70;
    $y = 150;
    foreach (str_split('Hello World!') as $char) {
        $glyph = $font[$char] ?? $font[' '];
        foreach ($glyph as $row => $bits) {
            for ($col = 0; $col < 5; $col++) {
                if ($bits[$col] !== '1') {
                    continue;
                }
                for ($dy = 0; $dy < $scale; $dy++) {
                    for ($dx = 0; $dx < $scale; $dx++) {
                        SDL_RenderDrawPoint($renderer, $x + $col * $scale + $dx, $y + $row * $scale + $dy);
                    }
                }
            }
        }
        $x += 6 * $scale; // 5px glyph + 1px gap
    }

    // Present the frame and keep the window up briefly while pumping events.
    SDL_RenderPresent($renderer);
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        SDL_PumpEvents();
        SDL_Delay(16);
    }
} finally {
    if (!is_null($renderer)) {
        SDL_DestroyRenderer($renderer);
    }
    // Destroy the window if creation succeeded.
    if (!is_null($window)) {
        SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        SDL_Quit();
    }
}
```

## Generated Files

Each generated PHP/Rust file starts with a header comment that records:

- the generation timestamp,
- the host it was generated on,
- the generator's OS/architecture,
- the PHP version.

Generated package files may be overwritten whenever they're regenerated. If you want to change behavior by hand, add overrides under `src/` instead of editing `src/generated` directly.

## Validation And Development

Quick checks:

```sh
# Check Rust formatting.
cargo fmt --check

# Run the Rust tests.
cargo test

# Run the PHPUnit tests.
composer test

# Run php-cs-fixer in check mode.
composer cs

# Run PHPStan.
composer analyse

# Validate the project config and generated state.
pnl validate
```

A smoke test to confirm install works end to end:

```sh
# Install libusb from the repository.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# Validate the project after installing.
pnl validate

# Rebuild the installed libusb bridge.
pnlx build libusb

# Run the libusb sample from the PHP Usage section.
php <libusb-example.php>
```

Syntax-check the generated PHP:

```sh
# Syntax-check every generated PHP file under @pnlx/packages.
find @pnlx/packages -name '*.php' -print0 | xargs -0 -n1 php -l
```

## Schemas

Each JSON file's format is versioned by `schema_version`. The current schemas are OpenAPI 3.0.3 documents, located at:

```text
schemas/pnl/2026-07-01/schema.json
schemas/pnlx/2026-07-01/schema.json
schemas/pnlx-lock/2026-07-01/schema.json
schemas/pnlx-pathmap/2026-07-01/schema.json
schemas/repository-index/2026-07-01/schema.json
```

Both the Rust CLI and the PHP SDK validate against these schemas before running their own domain validation.

## Limitations

- Repository-index resolution is not complete yet.
- FTP/FTPS install sources are recognized but not downloaded.
- Archive distribution download and extraction are not implemented.
- Repository-index signatures are not implemented.
- Macros and static inline C functions are not usable from PHP unless they are represented as linkable bridge functions.
- The lock/pathmap files are tied to a single environment. If the environment doesn't match, validation and runtime loading both error out.

## License

This repository is currently marked as MIT in `composer.json`. The bundled C libraries keep their own upstream licenses; see the package manifests and READMEs at `https://github.com/m3m0r7/pnl-packages`.
