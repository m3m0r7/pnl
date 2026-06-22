---
name: pnl-php-sdk
description: Understand the Pnlx PHP runtime SDK — Composer install (m3m0r7/pnl) and the lazy native binary, the @pnlx/autoload.php loading model, Pnlx\Runtime and feature flags, the all-static generated entity classes, the Pnlx\Types\* value layer, the Pnlx\Util\* wrapper-aware helpers, and the Pnlx\Exception\* hierarchy. Use when debugging FFI loading/marshalling, extending the runtime, or explaining how generated PHP calls reach C.
---

# The Pnlx PHP SDK (runtime)

The `Pnlx` PHP SDK is the runtime that opens the native library through PHP FFI and
forwards your calls into C. Source lives at `src/sdk/Pnlx/`; at install time `pnl` copies
it (plus the `libpnl.*` support library) into `@pnlx/runtime/`. Most callers never touch
the SDK directly — generated entity classes do it for you — but you need this model when
debugging loading/FFI/marshalling, or extending the runtime. For consumer-facing calling
conventions see the `pnl-usage` skill.

## Install & the native binary

```sh
composer require m3m0r7/pnl     # one package = the FFI runtime SDK + the pnl/pnlx CLIs
```

- Requires PHP `>=8.3`. It is an ordinary library — **no Composer plugin**, so `composer`
  never touches `allow-plugins` and your `composer.json` stays untouched.
- Installs `vendor/bin/pnl` and `vendor/bin/pnlx` like phpunit ships `vendor/bin/phpunit`.
- The native CLI binary is produced **lazily on first CLI run**: built from the bundled
  Rust sources if a Rust toolchain is present, else the matching prebuilt is downloaded
  from GitHub releases. Cached under `vendor/m3m0r7/pnl/bin/.native/<version>/`, so later
  runs exec it directly (`NativeBinaryLocator` resolves it).

## Loading model

One require boots everything:

```php
require_once __DIR__ . '/@pnlx/autoload.php';
```

`@pnlx/autoload.php` loads the SDK through its **own** autoloader (from `@pnlx/runtime/`),
so the runtime needs **no Composer autoloader**. It then loads every installed package's
generated entrypoint. If you also use Composer in the same process, require Composer's
`vendor/autoload.php` first (for your app/the SDK source), then `@pnlx/autoload.php` (for
the installed extensions).

## `Pnlx\Runtime` — the central entry point

`Pnlx\Runtime` (implements `RuntimeInterface`) wires the SDK together: on construction it
calls `Verifier::shouldEnabledFFI()` (fail fast if FFI is off), then assembles

- `RuntimeConfig` — project-root / `output_dir` resolution,
- `WorkspaceRepository` — reads `pnl.json` / `pnlx-lock.json` / `pnlx-pathmap.json`,
- `ExtensionRegistry` — locates installed packages,

and loads generated entrypoints + their `<Class>Manifest` metadata. Collaborators are
injectable for testing. Generated entrypoints may build a `new Runtime()` without an
explicit root; an internal active-project-root scope keeps nested loads tied to the
caller's project. You rarely instantiate it yourself.

Feature flags are read straight from `pnl.json` via static methods (no full runtime
needed) — these select which generated variant a class uses:

| Method | `features.*` key | Effect |
| --- | --- | --- |
| `Runtime::enableFunctions()` | `use_functions` | define `\Pnlx\Func\<Class>\*` global functions |
| `Runtime::allowCData()` | `allow_cdata` | accept raw `\FFI\CData` in signatures (`cdata/<Class>.php`) |
| `Runtime::useScalarsInParams()` | `use_php_scalars_in_params` | methods accept raw PHP scalars |
| `Runtime::useScalarsInReturn()` | `use_php_scalars_in_return` | methods return PHP scalars (`scalar/<Class>.php`) |
| `Runtime::useScalarsInConst()` | `use_php_scalars_in_const` | `const.php` uses PHP scalars (`scalar/const.php`) |

## Composing extensions into one FFI scope — `Runtime::compose()`

Normally each extension boots its **own** `NativeLibrary` (its own `FFI::cdef` scope),
so a `\FFI\CData` returned by one package can't be passed into another's function (PHP
FFI rejects cross-scope CData). `Runtime::compose([A::class, B::class, …])` fixes that for
a set of generated extensions:

```php
use Pnlx\Runtime;
Runtime::compose([Libsdl::class, Sdlimage::class]);          // call once, before using them
$surface = Sdlimage::IMG_Load('logo.svg');                   // CData from one package…
$texture = Libsdl::SDL_CreateTextureFromSurface($r, $surface); // …consumed by another — OK
```

It merges the members' generated cdefs (`Pnlx\FFI\CdefComposer`: the first member's cdef is
kept whole, later members contribute only declarations whose introduced C identifier is new
— so co-libraries that forward-declare a shared type and add their own functions merge
without redeclaration), co-loads every member library into one scope
(`NativeLibrary::composite()`), and adopts that scope into each member class
(`AbstractExtension::pnlxAdoptNativeLibrary()`). The members' generated methods keep
marshalling and return-wrapping unchanged; only the underlying native scope is now shared.

It returns a `Pnlx\ComposedScope` that also proxies calls
(`Runtime::compose([...])->some_c_function(...)`), routing each to the member that exposes
it. This is the consumer-side counterpart to a package's authoring-time `dependencies`
co-load (which only resolves symbols *within* one package's cdef). Distinct from the
single-package multi-library `LIBRARIES` co-load.

## Entity classes (generated)

Each package's entity is `Pnlx\<Pkg>\<Class>` extending `Pnlx\Extension\AbstractExtension`.
It is **all-static, never instantiated**. A magic `__callStatic` (in the base class)
performs the one-time boot (`PNLX_BOOT_TOKEN`) and forwards calls into FFI, so the class
body holds only methods named after C functions plus metadata constants:
`NAME`, `VERSION`, `DESCRIPTION`, `PATH`, `HASH`, `LIBRARIES` (co-loaded deps). Generated
methods/classes carry the `#[\Pnlx\Attribute\AutoGeneratedByPnlx]`,
`NativeLibraryName`, `NativeLibraryVersion`, and `RawNativeName` attributes.

## The type layer — `Pnlx\Types\*`

Self-contained value types shipped with the SDK (resolved by the SDK autoloader), e.g.
`Int_`, `Float_`, `Double`, `String_`, `Bool_`, `Null_`, `SizeT`, `SsizeT`, `TimeT`,
`ClockT`, `WcharT`, `Long`, `LongLong`, `Short`, signed/unsigned int 8/16/32/64, plus
abstractions `AbstractInteger`, `AnySizeInteger`, `AnyFloat`, and `PointerInterface`.
Without the `use_php_scalars_in_*` features, generated methods take/return these wrappers
instead of plain PHP scalars.

## Util helpers — `Pnlx\Util\*` (functions)

Wrapper-aware replacements for PHP built-ins (`src/sdk/Pnlx/Util/functions.php`):

- `is_null($v)` — true for `Null_`, unwraps `PointerInterface` then `FFI::isNull()`, else
  PHP `\is_null`. **Import it to shadow the built-in** when mixing native + plain values:
  `use function Pnlx\Util\is_null;`
- `is_int` / `is_integer` / `is_long`, `is_float` / `is_double`, `is_string` — treat the
  matching `Pnlx\Types\*` wrappers as their scalar type.
- `gettype($v)` — reports wrapped values as `integer` / `double` / `string`.
- `is_native_class($classOrObj)`, `is_native_function($fnOrMethod)` — whether something
  was generated by pnl (checks the `AutoGeneratedByPnlx` attribute).

## Exceptions — `Pnlx\Exception\*`

- `FFIUnavailableException` — PHP FFI is disabled/unavailable (`ffi.enable`).
- `ExtensionLoadException` — entrypoint or `<Class>Manifest` not defined after loading.
- `NativeFunctionCallException` — a forwarded FFI call failed.
- `UnsupportedNativeFunctionException` — the symbol/signature isn't callable.
- `PHPNativeLibraryException` — base for native-library problems.

## FFI internals (for debugging)

Under `Pnlx\FFI\`: `NativeLibrary` (opens the `.so`/`.dylib`/`.dll`), `ArgumentMarshaller`
+ `OutParameterMarshaller` (convert PHP ↔ C args, including by-ref out-params),
`Allocator` / `AllocationType`, `SymbolMode` / `SymbolInterface`, `FFIVerifier`. Schema
validation (`Pnlx\Schema\SchemaValidator`) is backed by `@pnlx/runtime/libpnl.*` and
no-ops when that library is absent (files were already validated at install).

## Developing the SDK in this repo

```sh
composer test              # PHPUnit
composer cs                # php-cs-fixer (dry-run); composer cs:fix to apply
composer analyse           # PHPStan (max level)
composer validate:schemas  # assert every schemas/*/*/schema.json is valid OpenAPI
```

`config.platform.php` is pinned to `8.3.0` in `composer.json`. The SDK source is
`src/sdk/Pnlx/`; the same tree is what ends up in `@pnlx/runtime/Pnlx/` after install.
