# pnl

`pnl` is a prototype package manager and runtime toolchain for PHP native-library extensions. It installs extension packages, resolves native shared libraries and headers, generates PHP FFI wrappers, compiles Rust bridge libraries, and exposes the generated wrappers through the `Pnlx` PHP SDK.

The project is split into two command-line tools:

- `pnl`: project-side installer, lockfile manager, validator, and package lister.
- `pnlx`: extension-authoring tool for generating wrappers and rebuilding installed Rust bridges.

Generated install state is written under `@pnlx/`, not Composer's `vendor/`. PHP code loads Composer first for the SDK, then `@pnlx/autoload.php` for installed native extensions.

## Status

This repository is an early implementation. Local path, `file://`, and git-based installs work for package roots that contain `pnlx.json`. Repository index solving, signed package indexes, archive downloaders, and FTP download support are still design-stage work.

## Requirements

Minimum practical requirements:

| Tool | Required | Notes |
| --- | --- | --- |
| Rust | 1.85+ recommended | Uses Rust 2024 edition. `rustc` is also used to compile bridge cdylibs. |
| PHP CLI | 8.2+ recommended | The `ffi` extension must be loaded and `ffi.enable` must allow CLI FFI. |
| Composer | 2.x | Installs PHPUnit, PHPStan, php-cs-fixer, and `cebe/php-openapi`. |
| Git | 2.x | Required for git install sources. |
| Make | any POSIX make | Used by the included `Makefile`. |
| pkg-config | optional but recommended | Used to discover native library versions and include paths. |
| Native libraries | package-specific | libusb/libnfc/SDL examples require matching libraries and headers. |

Current validated local environment:

```text
rustc 1.92.0
cargo 1.92.0
PHP 8.5.2 CLI NTS
Composer 2.8.8
git 2.52.0
pkg-config 2.5.1
macOS/aarch64 with Homebrew libusb, libnfc, SDL2
```

PHP FFI checks are performed at runtime. If PHP cannot use FFI, the SDK raises an FFI-related exception before loading a bridge.

## Build And Install

Build release binaries:

```sh
# Build both CLI binaries in release mode.
make build
```

Expected result:

```text
cargo build --release --bins
target/release/pnl
target/release/pnlx
```

Install binaries to a prefix:

```sh
# Install the release binaries into /usr/local/bin.
make install PREFIX=/usr/local
```

Expected result:

```text
install -d "/usr/local/bin"
install -m 0755 target/release/pnl "/usr/local/bin/pnl"
install -m 0755 target/release/pnlx "/usr/local/bin/pnlx"
```

For local development you can call `target/debug/pnl` and `target/debug/pnlx` after `cargo build`, but user-facing examples below assume installed `pnl` and `pnlx` binaries.

## Project Layout

After installing extensions, a PHP project looks like this:

```text
project-root/
  composer.json
  pnl.json
  @pnlx/
    autoload.php
    pnlx-lock.json
    pnlx-pathmap.json
    packages/
      vendor/
        package/
          pnlx.json
          src/generated/
    bridges/
      library-key/
        crate.rs
        <package>.bridge.rs
        lib<package>_bridge.dylib
```

Important files:

- `pnl.json`: editable project manifest.
- `@pnlx/pnlx-lock.json`: generated lockfile for the current platform.
- `@pnlx/pnlx-pathmap.json`: generated native/header/bridge path map for the current platform.
- `@pnlx/autoload.php`: generated PHP entrypoint requiring each installed package's generated `index.php`.

## Writing `pnl.json`

`pnl.json` is the project-side manifest. It says where extension packages may be discovered from, where native libraries may be searched, whether optional runtime features are enabled, and which extensions the project wants installed.

Minimal manifest:

```jsonc
{
  // Schema version used to select schemas/pnl/<version>/schema.json.
  "schema_version": "2026-07-01",
  // Repository indexes are optional when installing directly from a URL.
  "repositories": [],
  // Extra native library search paths.
  "load_paths": [],
  // Runtime feature flags.
  "enables": {
    // Keep generated global functions disabled by default.
    "use_functions": false
  },
  // Installed extension constraints keyed by vendor/package.
  "extensions": {}
}
```

Typical local-development manifest:

```jsonc
{
  // Schema version for this pnl.json document.
  "schema_version": "2026-07-01",
  // Optional repository sources for package indexes.
  "repositories": [
    {
      // Local file repository.
      "type": "file",
      "url": "file://packages"
    }
  ],
  // Native library directories checked before system defaults.
  "load_paths": [
    "/opt/homebrew/lib",
    "/usr/local/lib"
  ],
  // Enable generated C-style PHP functions.
  "enables": {
    "use_functions": true
  },
  // Extension requirements for this project.
  "extensions": {
    "libusb/libusb": {
      // Accept libusb wrapper versions in the 1.x line.
      "version": ">=1.0.0 <2.0.0",
      // Treat this extension as required.
      "required": true
    }
  }
}
```

Fields:

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `schema_version` | string | yes | Manifest schema version. Current value is `2026-07-01`. |
| `repositories` | array | yes | Package index/discovery sources. Currently useful for file repository fallback; full index solving is not complete. |
| `load_paths` | array | yes | Extra native library search paths checked before system defaults and environment-derived paths. |
| `enables.use_functions` | boolean | yes | When `true`, generated entrypoints may define global PHP functions for native C function names. |
| `extensions` | object | yes | Desired extension packages keyed by `vendor/package`. `pnl install` adds entries here. |

Repository entries:

```jsonc
// Local package index.
{ "type": "file", "url": "file://packages" }

// Git-backed package index.
{ "type": "git", "url": "git@github.com:vendor/pnl-index.git" }

// HTTPS package index with a reserved signing key field.
{ "type": "https", "url": "https://example.com/pnl/index.json", "key": "ed25519:<public-key>" }
```

`type` can be `file`, `git`, or `https`. `key` is optional and reserved for signed repository indexes. Direct install does not require a repository entry when you pass a local path, `file://` URL, or git URL directly to `pnl install`.

`load_paths` are native library directories, not include directories. Header search uses `pkg-config`, C include environment variables, package-local includes, and common system include directories.

Extension constraints:

```jsonc
// Extension constraints are stored under the top-level "extensions" object.
"extensions": {
  // Package key in vendor/package form.
  "vendor/package": {
    // Exact, range, caret, and tilde constraints are supported.
    "version": "^1.2.3",
    // Required extensions are expected to be present for this project.
    "required": true
  }
}
```

Supported version constraint forms include exact versions, comparison ranges, caret, tilde, and AND ranges such as `>=1.0.0 <2.0.0`. `required` is currently metadata for dependency intent; install still expects explicitly requested sources in this MVP.

Global function mode:

```jsonc
// Runtime feature flags are stored under the top-level "enables" object.
"enables": {
  // true lets generated entrypoints define global C-style PHP functions.
  "use_functions": true
}
```

When enabled, generated package entrypoints can emit functions such as `libusb_init()` if those names do not already exist. The generated functions delegate to a runtime-loaded entity through `$GLOBALS[...]`. When disabled, you call methods through `$runtime->load(...)`.

## Install Sources

`pnl install` accepts extension sources, not only `packages/<name>` paths.

Supported now:

```sh
# Install from a local extension root.
pnl install /absolute/path/to/extension-root

# Install from a file:// URL.
pnl install file:///absolute/path/to/extension-root

# Install from a package directory inside a GitHub repository.
pnl install https://github.com/m3m0r7/pnl-packages/packages/libusb

# Install from a GitHub web tree URL; "main" becomes the clone branch.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# Install from an scp-like SSH URL with a package subdirectory.
pnl install git@github.com:m3m0r7/pnl-packages/packages/libusb

# Install from a Git repository whose root contains pnlx.json.
pnl install git@github.com:vendor/repository.git
```

For GitHub HTTPS URLs and scp-like SSH URLs, the first two path segments after the host are treated as `owner/repository`; the remaining path is treated as the package root inside that repository. GitHub web URLs using `/tree/<branch>/...` are also accepted; the `<branch>` segment is used as the clone branch and the remaining path is used as the package root. For example:

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

The clone checkout is temporary and lives under the system temp directory, for example `/tmp/pnl/git/...`. Only the resolved package directory containing `pnlx.json` is copied into `@pnlx/packages/<vendor>/<package>`.

Root-repository installs still work:

```sh
# Install from a root repository that contains pnlx.json.
pnl install git@github.com:sunng87/handlebars-rust.git
```

In every case, install succeeds only if the resolved local source path contains `pnlx.json`.

FTP/FTPS targets are recognized but fail explicitly until a downloader and signature verification path are implemented.

## pnl Commands

### `pnl help`

Prints the command summary and available subcommands.

```sh
# Show the generated command summary and subcommands.
pnl help
```

### `pnl version`

Prints the current `pnl` binary version.

```sh
# Print the binary version from Cargo.toml.
pnl version
```

Example output:

```text
0.1.0
```

### `pnl init`

Creates a default `pnl.json` if it does not already exist.

```sh
# Create pnl.json when it does not already exist.
pnl init
```

Example output:

```text
initialized ./pnl.json
```

Result:

```jsonc
{
  // Schema version for pnl.json validation.
  "schema_version": "2026-07-01",
  // No repository indexes are configured by default.
  "repositories": [],
  // No extra native library directories are configured by default.
  "load_paths": [],
  // Global PHP function generation is disabled by default.
  "enables": {
    "use_functions": false
  },
  // No extensions are installed yet.
  "extensions": {}
}
```

### `pnl install <source>`

Installs an extension source, resolves native libraries and headers, generates PHP/Rust wrapper files, compiles the bridge, updates `pnl.json`, writes lock/pathmap files, and regenerates `@pnlx/autoload.php`.

```sh
# Install libusb from the package repository's main branch.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
```

Example output:

```text
generated ./@pnlx/packages/libusb/libusb/src/generated/libusb.ffi.php
generated ./@pnlx/packages/libusb/libusb/src/generated/Libusb.php
generated ./@pnlx/packages/libusb/libusb/src/generated/LibusbContext.php
generated ./@pnlx/packages/libusb/libusb/src/generated/index.php
generated ./@pnlx/packages/libusb/libusb/src/generated/function.aliases.php
generated ./@pnlx/packages/libusb/libusb/src/generated/libusb.bridge.rs
installed extension libusb/libusb
```

Resulting state:

```text
@pnlx/packages/libusb/libusb/
@pnlx/bridges/libusb-1.0/libusb_bridge.dylib
@pnlx/pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
```

### `pnl update [vendor/package]`

Re-installs one package from its lockfile source, or all installed packages when no package is provided.

```sh
# Reinstall one extension from its locked source URL.
pnl update libusb/libusb

# Reinstall every installed extension from its locked source URL.
pnl update
```

Expected behavior:

- Reads `@pnlx/pnlx-lock.json`.
- Reuses each locked `source.url`.
- Re-runs install, generation, bridge compilation, and pathmap updates.

### `pnl uninstall <vendor/package>`

Removes an extension from `pnl.json`, removes its lock entry, deletes the installed package directory, and regenerates `@pnlx/autoload.php`.

```sh
# Remove libusb from pnl.json, lock/pathmap state, and @pnlx/packages.
pnl uninstall libusb/libusb
```

Example output:

```text
uninstalled libusb/libusb
```

### `pnl list`

Lists installed extensions. `pnl list` defaults to `pnl list extensions`.

```sh
# List installed extensions.
pnl list

# Same as "pnl list".
pnl list extensions
```

Example output:

```text
libnfc/libnfc 1.8.0 1.8.0
libsdl/libsdl 2.32.10 2.32.10
libusb/libusb 1.0.29 1.0.29
```

### `pnl list native`

Lists resolved native libraries from `@pnlx/pnlx-pathmap.json`.

```sh
# Show resolved native libraries and their filesystem paths.
pnl list native
```

Example output:

```text
libnfc 1.8.0 /opt/homebrew/lib/libnfc.dylib
libusb-1.0 1.0.29 /opt/homebrew/lib/libusb-1.0.dylib
sdl2 2.32.10 /opt/homebrew/lib/libSDL2.dylib
```

### `pnl list repos`

Lists repositories configured in `pnl.json`.

```sh
# Add a local file repository entry.
pnl repo add file file:///tmp/pnl-packages-demo

# Print configured repository entries.
pnl list repos
```

Example output:

```text
File file://packages
File file:///tmp/pnl-packages-demo
```

### `pnl repo add <git|file|https> <url> [--key <key>]`

Adds an extension index repository to `pnl.json`. Duplicate URLs are ignored.

```sh
# Add a local index directory.
pnl repo add file file:///absolute/path/to/index

# Add a Git index repository.
pnl repo add git git@github.com:vendor/pnl-index.git

# Add an HTTPS index and reserve a public key for future signature checks.
pnl repo add https https://example.com/pnl/index.json --key ed25519:<public-key>
```

Result:

```jsonc
{
  // Repository kind.
  "type": "file",
  // Repository URL stored in pnl.json.
  "url": "file:///absolute/path/to/index"
}
```

Repository index resolution is not complete yet; this command records configuration for the planned resolver and for runtime fallback discovery of local file repositories.

### `pnl repo remove <url>`

Removes a repository entry from `pnl.json`.

```sh
# Remove a repository URL from pnl.json.
pnl repo remove file:///absolute/path/to/index
```

No output is printed on success.

### `pnl validate`

Validates `pnl.json`, `@pnlx/pnlx-lock.json`, and `@pnlx/pnlx-pathmap.json` when present.

```sh
# Validate pnl.json plus generated lock/pathmap files when present.
pnl validate
```

Example output:

```text
pnl workspace is valid
```

Validation includes:

- OpenAPI schema validation by `schema_version`.
- package name and SemVer checks.
- platform match checks for lock/pathmap files.
- pathmap/lock domain checks.

### `pnl self-upgrade`

Reserved command. It currently returns an implementation error.

## pnlx Commands

### `pnlx help`

Prints the command summary and available subcommands.

```sh
# Show the generated command summary and subcommands.
pnlx help
```

### `pnlx version`

Prints the current `pnlx` binary version.

```sh
# Print the binary version from Cargo.toml.
pnlx version
```

Example output:

```text
0.1.0
```

### `pnlx init`

Creates a default `pnlx.json` in an extension package root if it does not exist.

```sh
# Create a new package workspace directory.
mkdir -p packages/example

# Enter the package workspace.
cd packages/example

# Create pnlx.json when it does not already exist.
pnlx init
```

Example output:

```text
initialized ./pnlx.json
```

### `pnlx validate`

Validates an extension workspace's `pnlx.json`.

```sh
# Clone the package repository for package-authoring commands.
git clone https://github.com/m3m0r7/pnl-packages.git

# Enter the libusb package root.
cd pnl-packages/packages/libusb

# Validate pnlx.json and package-domain values.
pnlx validate
```

Example output:

```text
pnlx workspace is valid
```

### `pnlx gen <target> [--library-key <key>]`

Generates extension artifacts under `src/generated` for an extension package.

```sh
# Clone the package repository for package-authoring commands.
git clone https://github.com/m3m0r7/pnl-packages.git

# Enter the libusb package root.
cd pnl-packages/packages/libusb

# Generate FFI CDEF, entity/context, aliases, entrypoint, and bridge Rust.
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

Behavior:

- Reads `pnlx.json`.
- Resolves headers from `@pnlx/pnlx-pathmap.json` when run inside an installed project.
- Falls back to package `headers` entries when pathmap headers are absent.
- Generates PHP entity/context classes, PHPDoc/wrapper methods, aliases, CDEF, entrypoint, and Rust bridge source.

Use `--library-key` when a package has multiple native requirements and the target name is ambiguous:

```sh
# Generate artifacts for a target whose native requirement key is explicit.
pnlx gen libfoo --library-key libfoo-2.0
```

### `pnlx build [vendor/package ...]`

Rebuilds compiled Rust bridge libraries for installed packages.

```sh
# Build every installed bridge.
pnlx build

# Build by unambiguous package leaf.
pnlx build libusb

# Build multiple installed bridges.
pnlx build libusb libnfc libsdl

# Build by full vendor/package name.
pnlx build libusb/libusb
```

Example output:

```text
built 3 bridge(s)
```

Behavior:

- Reads `@pnlx/pnlx-lock.json`.
- Reads native paths from `@pnlx/pnlx-pathmap.json`.
- Compiles installed `src/generated/*.bridge.rs` with `rustc --crate-type cdylib`.
- Writes compiled libraries under `@pnlx/bridges/<library-key>/`.
- Updates `bridges` entries in `@pnlx/pnlx-pathmap.json`.

### `pnlx package`

Reserved command. It currently returns an implementation error.

## PHP Usage

Install example packages from the package repository:

```sh
# Install libusb, libnfc, and SDL package wrappers.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libnfc
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libsdl

# Rebuild compiled Rust bridge libraries after install.
pnlx build
```

### `test.php`

libusb version/error/device count example:

```php
<?php

declare(strict_types=1);

// Run the sample from the project root regardless of the caller's cwd.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libusb\Libusb;
use Pnlx\Runtime;

// Runtime resolves manifests, pathmaps, generated entrypoints, and bridge FFI.
$runtime = new Runtime(__DIR__);

/** @var Libusb $libusb */
// Load the generated libusb entity through Runtime.
$libusb = $runtime->load(Libusb::class);

// Read generated package metadata and the compiled bridge path.
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

Example `php test.php` output:

```text
extension: libusb/libusb 1.0.29
bridge: /path/to/project/@pnlx/bridges/libusb-1.0/libusb_bridge.dylib
error name for 0: LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED
strerror for 0: Success
libusb_init: 0 (LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED)
device count: 6
libusb_exit: ok
```

### `test2.php`

SDL window through generated entity methods:

```php
<?php

declare(strict_types=1);

// Run the sample from the project root regardless of the caller's cwd.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use Pnlx\Runtime;
use Pnlx\Util;

// SDL video subsystem flag.
const SDL_INIT_VIDEO = 0x00000020;

// Ask SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Create a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

// Runtime loads the generated SDL entity and its compiled bridge.
$runtime = new Runtime(__DIR__);

/** @var Libsdl $sdl */
// Use generated entity methods such as SDL_Init() and SDL_CreateWindow().
$sdl = $runtime->load(Libsdl::class);

// Keep handles outside the try block so cleanup can see them.
$window = null;
$initialized = false;

try {
    // Start SDL's video subsystem.
    $result = $sdl->SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . $sdl->SDL_GetError());
    }
    $initialized = true;

    // Create a small Hello World window.
    $window = $sdl->SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );

    if (Util::isNull($window)) {
        // Util::isNull() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . $sdl->SDL_GetError());
    }

    echo "Hello World!\n";

    // Make the title explicit and present the window surface.
    $sdl->SDL_SetWindowTitle($window, 'Hello World!');
    $sdl->SDL_ShowWindow($window);
    $sdl->SDL_UpdateWindowSurface($window);

    // Keep the window alive briefly while pumping SDL events.
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        $sdl->SDL_PumpEvents();
        $sdl->SDL_Delay(16);
    }
} finally {
    // Destroy the SDL window if creation succeeded.
    if (!Util::isNull($window)) {
        $sdl->SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        $sdl->SDL_Quit();
    }
}
```

### `test3.php`

SDL window through generated global functions:

```php
<?php

declare(strict_types=1);

// Run the sample from the project root regardless of the caller's cwd.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use Pnlx\Runtime;
use Pnlx\Util;

// SDL video subsystem flag.
const SDL_INIT_VIDEO = 0x00000020;

// Ask SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Create a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

// Loading the entity also loads generated global functions when enabled.
$runtime = new Runtime(__DIR__);
$runtime->load(Libsdl::class);

if (!function_exists('SDL_Init')) {
    // Global functions are controlled by pnl.json enables.use_functions.
    throw new RuntimeException('SDL global functions are disabled. Set pnl.json enables.use_functions to true.');
}

// Keep handles outside the try block so cleanup can see them.
$window = null;
$initialized = false;

try {
    // Start SDL's video subsystem through the generated global function.
    $result = SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . SDL_GetError());
    }
    $initialized = true;

    // Create a small Hello World window.
    $window = SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );

    if (Util::isNull($window)) {
        // Util::isNull() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . SDL_GetError());
    }

    echo "Hello World!\n";

    // Make the title explicit and present the window surface.
    SDL_SetWindowTitle($window, 'Hello World!');
    SDL_ShowWindow($window);
    SDL_UpdateWindowSurface($window);

    // Keep the window alive briefly while pumping SDL events.
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        SDL_PumpEvents();
        SDL_Delay(16);
    }
} finally {
    // Destroy the SDL window if creation succeeded.
    if (!Util::isNull($window)) {
        SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        SDL_Quit();
    }
}
```

## Generated Files

Generated PHP and Rust files contain a header comment with:

- generation timestamp
- generator host
- generator OS/architecture
- PHP version

Generated package files can be overwritten. Add manual overrides under source `src/` instead of editing `src/generated` directly.

## Validation And Development

Fast checks:

```sh
# Check Rust formatting.
cargo fmt --check

# Run Rust tests.
cargo test

# Run PHPUnit tests.
composer test

# Run php-cs-fixer in check mode.
composer cs

# Run PHPStan.
composer analyse

# Validate project manifests and generated install state.
pnl validate
```

Package install smoke:

```sh
# Install libusb from the package repository.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# Validate the project after install.
pnl validate

# Rebuild the installed libusb bridge.
pnlx build libusb

# Run the libusb PHP smoke example.
php test.php
```

Syntax check generated PHP:

```sh
# Syntax-check every generated PHP file under @pnlx/packages.
find @pnlx/packages -name '*.php' -print0 | xargs -0 -n1 php -l
```

## Schemas

JSON formats are versioned by `schema_version`. Current schemas are OpenAPI 3.0.3 documents under:

```text
schemas/pnl/2026-07-01/schema.json
schemas/pnlx/2026-07-01/schema.json
schemas/pnlx-lock/2026-07-01/schema.json
schemas/pnlx-pathmap/2026-07-01/schema.json
schemas/repository-index/2026-07-01/schema.json
```

Both Rust CLI and PHP SDK validation use these schema documents before applying domain validation.

## Limitations

- Repository index resolution is not complete.
- FTP/FTPS install sources are detected but not downloaded.
- Archive dist download and extraction are not implemented.
- Repository index signatures are not implemented.
- Macro/static inline C functions are not exposed unless they are represented by linkable bridge functions.
- Lock/pathmap files are single-platform. Platform mismatch is an error for validation and runtime loading.

## License

This repository is currently marked as MIT in `composer.json`. Packaged native libraries keep their upstream licenses; see the package manifests and README files in `https://github.com/m3m0r7/pnl-packages`.
