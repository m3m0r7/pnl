---
name: pnl-usage
description: Use the `pnl` CLI and the generated `Pnlx` PHP SDK to install C libraries and call them from PHP. Covers pnl.json, the install/lock/validate flow, the @pnlx layout, and the PHP calling conventions (static methods vs global functions, pointers/structs/out-params, scalar wrappers, null checks, constants). Use when installing a C library package, wiring up pnl.json, or writing PHP that calls a generated extension.
---

# pnl usage (consuming C libraries from PHP)

`pnl` is "Composer for C libraries": it finds a native library + headers already on the
machine, generates PHP wrappers under `@pnlx/`, and exposes them through the `Pnlx` SDK.
`pnl` is for *using* libraries; `pnlx` (see the `pnlx-authoring` skill) is for *making* them.

## The flow

```sh
pnl init                 # create pnl.json (idempotent)
pnl install libc         # bare name → resolved from default repo github.com/m3m0r7/pnl-packages
pnl install              # no source → restore everything from pnlx-lock.json (sha256-verified)
pnl list 'lib*'          # installed extensions (glob matches vendor/pkg and leaf)
pnl search 'lib*'        # packages available from repos
pnl validate             # check pnl.json + lock + pathmap consistency
```

Install sources accepted by `pnl install`: bare name, local path, `file://`, GitHub
HTTPS/`/tree/<branch>/` URL, scp-style SSH URL, or a `.tar.gz`/`.tgz`/`.zip` archive
(local or remote). Pin with `@<version>` (e.g. `pnl install libusb@1.0.29`). Multiple
sources in one call are fine. If the package declares an `installation` recipe (e.g.
`brew install`), install offers to run it — `-y` to auto-accept, `-n` for non-interactive.

Useful flags: `--alias-class <Class>`, `--function-prefix <prefix>`, `-f/--force`
(overwrite locked digest on content mismatch).

## Files you touch vs files that are generated

- **`pnl.json`** — the config you edit (see below). Commit it.
- **`pnlx-lock.json`** — lockfile pinning versions + content sha256. Sits next to
  `pnl.json` (not under `@pnlx/`). Commit it.
- **`@pnlx/`** — entirely generated; do not hand-edit. Contains `autoload.php`,
  `pnlx-pathmap.json` (where the C libs/headers live on this machine), `runtime/`
  (the bundled SDK + `libpnl.*`), and `packages/<vendor>/<pkg>/<version>/src/generated/`.
  To override generated behavior, add code under `src/` rather than editing
  `src/generated`.

`output_dir` in pnl.json can move `@pnlx` elsewhere (default `@pnlx`).

## pnl.json essentials

```jsonc
{
  "schema_version": "2026-07-01",
  "repositories": [],        // extra package indexes; default repo is built-in
  "load_paths": [],          // extra folders to search for .so/.dylib (NOT headers)
  "features": {
    "use_functions": false,            // generate \Pnlx\Func\<Class>\* global functions
    "allow_cdata": false,              // params also accept raw \FFI\CData
    "use_php_scalars_in_params": true, // pass plain int/float/string instead of \Pnlx\Types\* wrappers
    "use_php_scalars_in_return": false,
    "use_php_scalars_in_const": false
  },
  "extensions": {
    "libusb/libusb": { "version": ">=1.0.0 & <2.0.0", "required": true }
  }
}
```

Version constraints: exact (`1.2.3` = `=1.2.3`), comparators, caret `^`, tilde `~`,
combined with `&` (binds tighter) and `|`, grouped with `()`. e.g. `>=1.0.0 & <2.0.0`.

`load_paths` are **library** folders only. Headers are found via libraries' `.pc`
files (parsed directly — no `pkg-config` binary needed), C include env vars, and system
include dirs. A package can also fetch its lib/header remotely (`library_url`,
`header_url`, `header_inline`) — that's declared in the package's `pnlx.json`, not here.

Override built-in endpoints per project under `config` (`self_repository`,
`packages_repository`) when self-hosting a fork/registry.

## Calling from PHP

One require loads the SDK and every installed extension:

```php
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libc\Libc;

Libc::printf("Hello from libc\n");   // static calls; first call boots the extension
```

A C library is a bag of functions → call **static methods** on the entity class
`Pnlx\<Pkg>\<Class>`. Method names mirror C names; camelCase aliases also exist
(`Libusb::libusb_error_name` ≡ `Libusb::libusbErrorName`).

Install-time metadata is exposed as constants: `Class::NAME`, `Class::VERSION`,
`Class::PATH`, `Class::HASH`, `Class::DESCRIPTION`.

### Global functions (optional)

Set `features.use_functions: true`, then functions live under `\Pnlx\Func\<Class>`:

```php
use function Pnlx\Func\Libsdl\{SDL_Init, SDL_Quit};
SDL_Init(SDL_INIT_VIDEO);
```

They're only defined if no same-named function already exists. Guard with
`function_exists('Pnlx\\Func\\Libsdl\\SDL_Init')` if a caller might run with the feature off.

### Pointers, structs, out-parameters

The bindings map C's pointer/value distinction onto PHP — no manual allocator:

- **Pointer param `T *` = by-reference out param.** Pass a variable; read it after:
  - `int *` scalar out → pass a variable (or `\Pnlx\Types\Int_`), read back
  - `char **` → you get a PHP string
  - `T **` handle out → you get a wrapped handle
  ```php
  $major = 0; $minor = 0; $rev = 0;
  Libgit2::git_libgit2_version($major, $minor, $rev);
  ```
- **A struct you own → `new`.** Types live under `\Pnlx\<Pkg>\Types\<struct>`; it decays
  to the pointer the C API wants:
  ```php
  use Pnlx\Libconfig\Types\config_t;
  $cfg = new config_t();
  Libconfig::config_init($cfg);
  ```
- **Writable `char *` buffer = by-reference.** Pre-size it, read bytes back:
  ```php
  $out = str_repeat("\0", 37);
  Libuuid::uuid_unparse($uuid, $out);
  echo rtrim($out, "\0");
  ```
- **Exported C data symbols** are flat marker classes; pass `::class`:
  ```php
  Liboniguruma::onig_get_syntax_options(\Pnlx\Liboniguruma\OnigDefaultSyntax::class);
  ```

Scalar value types are `\Pnlx\Types\*` (`Int_`, `Float_`, `String_`, …). With
`use_php_scalars_in_params`/`_return` you pass/get plain scalars; otherwise wrappers.

### Null checks and constants

Use `\Pnlx\Util\is_null($handle)` (wraps `FFI::isNull()` and understands wrappers) — not
PHP's built-in `is_null` — to test FFI handles. Constants are generated into the package's
`const.php`; import them (`use const Pnlx\Libsdl\SDL_INIT_VIDEO;`) rather than redefining.
Constant-argument macros (e.g. `SDL_WINDOWPOS_CENTERED`) are expanded and generated too.

## Verify an install end-to-end

```sh
pnl install <source> && pnl validate
find @pnlx/packages -name '*.php' -print0 | xargs -0 -n1 php -l   # syntax-check generated PHP
php your-example.php
```

## Gotchas

- Run scripts from project root or `chdir(__DIR__)` first — `@pnlx/autoload.php` and the
  pathmap are resolved relative to the workspace.
- The pathmap is environment-specific. After `git clone` on a new machine, run
  `pnl install` (no args) to restore packages and regenerate the pathmap.
- Generated files carry a header (timestamp, host, OS/arch, PHP version) and are
  overwritten on regen — never edit them.
- FFI must be enabled in PHP (`ffi.enable`), or the runtime throws
  `\Pnlx\Exception\FFIUnavailableException`.
