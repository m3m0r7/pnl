# Configuration

[← Documentation index](../../README.md) · [日本語](../ja/configuration.md)

## Project Layout

After you install extensions, a PHP project looks like this. Everything under `@pnlx/` is generated, so you normally don't edit it by hand:

```text
project-root/
  composer.json
  pnl.json                     ← the config file you edit
  pnlx-lock.json               ← the lockfile (commit it; not in @pnlx)
  @pnlx/                       ← everything below is generated
    autoload.php
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
- `pnlx-lock.json`: the lockfile that pins installed versions and content hashes. It sits next to `pnl.json` (not inside `@pnlx/`, whose location is configurable) so it has a fixed, committable path.
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
