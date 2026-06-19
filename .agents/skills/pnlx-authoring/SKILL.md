---
name: pnlx-authoring
description: Author a pnl extension package with the `pnlx` CLI — write pnlx.json (name/version/class/platforms/requires/dependencies), generate PHP wrappers with `pnlx gen`, stamp install-script hashes with `pnlx publish`, and index/sign a repository. Covers native-library discovery and remote header/library/inline sources. Use when creating or editing a package under pnl-packages, debugging wrapper generation, or wiring up requires/headers.
---

# pnlx authoring (making C-library packages)

`pnlx` is the tool for *authoring* packages — the thing consumers later `pnl install`.
For the consumer side see the `pnl-usage` skill. The canonical package set is
`github.com/m3m0r7/pnl-packages`; a package is any folder containing a `pnlx.json`.

## The flow

```sh
mkdir -p packages/example && cd packages/example
pnlx init                       # create pnlx.json
# edit pnlx.json: name, version, class, platforms, requires, headers
pnlx validate                   # schema + package-value checks
pnlx gen example                # generate src/generated/* (FFI defs, classes, aliases, entrypoint)
pnlx publish                    # stamp install_script_hash into pnlx.json before publishing
```

`pnlx gen <target>` reads `pnlx.json`, resolves headers from
`@pnlx/pnlx-pathmap.json` when run inside an installed project, and otherwise falls back
to the package's own `requires` header entries. It uses **libclang at runtime** to parse
headers — libclang must be installed on the generating machine. When one package needs
multiple C libraries and the target name is ambiguous, disambiguate with
`pnlx gen libfoo --library-key libfoo-2.0`.

Generated under `src/generated/`: `<class>.ffi.php`, `<Class>.php`,
`<Class>Context.php` (CData wrapper), `index.php` (entrypoint), `functions.php` +
`function.aliases.php`, `const.php`, `macro.functions.php`, plus `types/`, `cdata/`,
`scalar/`. These are overwritten on regen — put hand overrides under `src/`, not
`src/generated/`.

## pnlx.json essentials

```jsonc
{
  "schema_version": "2026-07-01",
  "name": "vendor/package",                 // vendor/package form
  "version": "1.2.3",                        // SemVer
  "description": "...",
  "authors": [{ "name": "..." }],
  "license": "MIT",
  "entrypoint": "src/generated/index.php",
  "class": "Pnlx\\Vendor\\Package",          // PHP entity class FQN
  "examples": ["EXAMPLES.md"],
  "platforms": [                             // os: darwin|linux|windows, arch: aarch64|x86_64
    { "os": "darwin", "arch": "aarch64" },
    { "os": "linux",  "arch": "x86_64" }
  ],
  "requires": { /* native library requirements, see below */ },
  "dependencies": {}                         // other pnl packages, resolved at install
}
```

## `requires` — native library discovery

Each key is a library requirement. By default `pnl install` finds the lib locally
(`load_paths`, `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`, `PATH`, system dirs) and the
header via `.pc` files / include paths. A requirement can override the source:

```jsonc
"requires": {
  "mylib-1.0": {
    "library_names": ["libmylib.so", "mylib.dll"],   // or [{ "name": "...", "virtual": true }]
    "symbol_prefix": "mylib_",
    "version": "^1.0.0",
    "required": true,

    // optional remote/inline sources (else local discovery is used):
    "library_url": "https://example.com/releases/libmylib.so", // http(s)|ftp|git tree URL
    "header_url":  "https://raw.githubusercontent.com/acme/mylib/v1.0/mylib.h",
    "header_inline": "int mylib_add(int a, int b);\n"          // embed header when no file exists
  }
}
```

- **`virtual: true`** library entries link by name without expecting a file on disk —
  used for libs that live in a shared cache (e.g. macOS libc in the dyld cache). See the
  `libc` package, which is all-virtual with a big `header_inline`.
- Supported remote schemes: `http`/`https`, `ftp`, `git` (`/tree/<branch>/<path>` or
  ssh/git/.git URL). `ftps` not implemented — use `https`/`ftp`. Remote assets are
  downloaded once and cached.
- `header_inline` is the simplest path for a small/portable API surface: paste the
  declarations directly and skip header resolution entirely.

## installation recipes & publish hashing

A package may declare an `installation` recipe (per-OS / per-distro commands like
`brew install`) or a `self_build` script (mutually exclusive with `installation`).
`pnl install` offers to run it. For trust, `pnlx publish` hashes those commands / the
referenced script and writes `install_script_hash` into `pnlx.json`:

- Without a matching hash, interactive installs default to "No"; `-y` installs stop.
- Consumers override with `--allow-install-script-hash <sha256>` or, as a last resort,
  `--allow-unverified-install-scripts`.
- First-party authorized repos skip the prompt entirely.
- `self_build` script paths must stay inside the package (no absolute paths, no `..`).
- On Linux the recipe key is chosen from `/etc/os-release`: distro `ID` first
  (`alpine`/`ubuntu`/`fedora`…), then each `ID_LIKE` ancestor, then a generic `linux` key.

Re-run `pnlx publish` whenever you change install/self_build commands.

## Repository indexing & signing (publishing a registry)

```sh
pnl repo index packages --base-url https://github.com/me/pnl-packages/tree/main/packages
pnl repo sign packages/repository-index.json --key ed25519:<base64-secret>
# consumers: pnl repo add https <url> --key ed25519:<base64-public>
```

`repository-index.json` lets `pnl search`/bare-name install enumerate without cloning;
each entry records versions, manifest path, `dist.sha256`, and an installable `source`
URL. Signed repos require a sibling `repository-index.json.sig` (Ed25519).

## Validate the generated PHP

```sh
pnlx validate
find src/generated -name '*.php' -print0 | xargs -0 -n1 php -l
```

## Notes

- Content integrity: on install, every file is sha256'd (sorted) and hashed into one
  `dist.sha256`; reinstalling the same version must match or it aborts. New versions are
  legitimate updates. Generated output, `.git`, and the workspace dir are excluded.
- Schemas live at `schemas/<kind>/<version>/schema.json` (OpenAPI 3.0.3). Both the Rust
  CLI and PHP SDK validate against them before domain checks. Bump `schema_version`
  together with the schema files.
- This repo's own `EXAMPLES.md` files are verified by actually running each library
  (workspace under `/tmp/pnlx-ex`); keep examples runnable, not aspirational.
