# Conventions

[← Documentation index](../../README.md) · [日本語](../ja/conventions.md)

These are the naming and coding rules for the **pnl source tree** (the Rust CLI,
the PHP SDK, the templates, and the JSON schemas). They describe the *intended*
state. Where the current code deviates, the deviation is a refactoring target,
not a counter-example. The planned v0.7.0 refactoring follows this document.

## Table of Contents

- [1. Design Principles](#1-design-principles)
- [2. Module Architecture](#2-module-architecture)
- [3. Rust Naming, Style, and Lints](#3-rust-naming-style-and-lints)
- [4. PHP SDK Naming and Style](#4-php-sdk-naming-and-style)
- [5. Code Generation and Templates](#5-code-generation-and-templates)
- [6. Schema Conventions](#6-schema-conventions)
- [7. Generated-Code Contract](#7-generated-code-contract)
- [8. Testing Conventions](#8-testing-conventions)

## 1. Design Principles

These override everything below; a stylistic rule never justifies breaking one.

- **Essential fixes only (本質的対応).** Solve the general problem, not the one
  failing library. **No per-library branching** in the generator, **no stopgaps**,
  and **no new dependency** unless the problem is genuinely unsolvable without it.
- **No-drop.** Never silently discard a function. If it cannot be bound (static
  inline with no symbol, unrepresentable type), still emit the method/function but
  **throw inside** it, tagged with the relevant attribute. The FFI `cdef` and the
  PHP method surface are separate, so a stub with no `cdef` entry does not break
  loading.
- **Schema is the source of truth.** Every config shape is defined by a JSON
  schema. The Rust CLI and the PHP SDK both conform to it; neither invents fields.
- **Generate, never concatenate.** All emitted PHP and C `cdef` text goes through
  Handlebars templates (`.tpl`). Rust never assembles target-language code with
  `format!`/string concatenation.
- **Deterministic output.** Iterate in a stable (sorted) order so generated files
  are byte-stable; this is what makes golden/snapshot tests meaningful.
- **Fail loud, fail early.** Prefer an actionable error over a silent fallback.
  Preflight what you can (toolchain checks). If coverage is bounded (a cap, a
  skip), **say so** — silent truncation reads as "covered everything".

## 2. Module Architecture

The crate is organized into **layers**. Imports flow in **one direction only**;
a lower layer never imports a higher one. `cli` is the only layer that
orchestrates the others.

```
util  ←  model  ←  { native, sources }  ←  codegen  ←  cli
                                                         embed (binary boundary)
```

| Layer        | Responsibility                                   | Modules (target location)                                                            |
|--------------|--------------------------------------------------|--------------------------------------------------------------------------------------|
| `util/`      | Domain-free helpers                              | `glob`, `io`                                                                          |
| `model/`     | Data types, validation, embedded schemas, config | `manifest`, `schema`, `validate`, `version`, `platform`, `config`, `workspace`, `repository_index` |
| `native/`    | C-world discovery and interop                    | `header_adapter` (split, see below), `pkg_config`, `native`, `cc`, `tbd`, `shim`, `install_script` |
| `sources/`   | Acquiring packages and assets                    | `fetch`, `archive`, `cache`, `git_source`                                            |
| `codegen/`   | Emitting PHP and `cdef`                          | `generate` (+ `types`, `php`, `names`, `aliases`), `templates`                       |
| `cli/`       | User-facing commands and presentation            | `commands/*`, `ui`, `interaction`, `about`, `highlight`, `self_upgrade`, `release`   |
| `embed/`     | The cdylib C ABI and embedded SDK payload        | `ffi`, `sdk_assets`                                                                  |

Rules:

- **One module, one purpose**, stated in a top-of-file module doc comment (`//!`).
  Every module must have one.
- **Split god-files.** A module over ~600 lines is a refactoring target. When a
  module grows into a directory, `mod.rs` holds only re-exports and wiring; the
  logic lives in named submodules. `header_adapter.rs` (the libclang parse, macro
  expander, and const evaluator) splits into a `native/header_adapter/` tree
  (e.g. `parse`, `macros`, `consts`, `types`); `install.rs` and `generate.rs`
  decompose the same way.
- **`native` is the only layer allowed to touch libclang, pkg-config files, or
  the C toolchain.** Nothing above it learns about C internals.
- New code goes in the layer matching its responsibility, not wherever is
  convenient. If it does not fit a layer, the layering is wrong — discuss before
  adding a catch-all.

## 3. Rust Naming, Style, and Lints

- **Formatting:** `cargo fmt` defaults. No `rustfmt.toml` unless a specific
  setting is required and justified in a comment.
- **Lints are in-tree, not just in CI.** Codify the deny level in `Cargo.toml`:

  ```toml
  [lints.rust]
  warnings = "deny"

  [lints.clippy]
  all = "deny"
  ```

  so a local `cargo build` enforces what CI enforces. CI keeps `-D warnings` as a
  backstop.
- **Names:** modules and functions `snake_case`, types/traits `CamelCase`,
  constants `SCREAMING_SNAKE_CASE`. Module names are **descriptive words**, not
  cryptic abbreviations. An established external name is acceptable only when it
  *is* the domain term (`tbd` = the `.tbd` stub format, `cc` = the `CC`
  convention); otherwise spell it out.
- **Errors:** `anyhow::Result` at boundaries with `.context(...)` carrying an
  actionable message. Introduce a typed error (`thiserror`) only when a caller
  must branch on the variant — today none do, so stay with `anyhow`.
- **No `unwrap`/`expect`** outside tests except where infallibility is provable,
  and then with a comment saying why.
- **Borrow over clone.** Heed `clippy::needless_clone` and friends; the deny
  level above makes them errors.

## 4. PHP SDK Naming and Style

- **Style (codified in `.php-cs-fixer.php`):** `@PSR12`, `declare(strict_types=1)`,
  `single_quote`, `ordered_imports`, `no_unused_imports`.
- **Static analysis:** PHPStan `level: max`, no baseline. New code lands clean.
- **Namespace:** everything under `Pnlx\`. One class or interface per file.
- **Interfaces:** named `<Name>Interface`; service classes depend on the
  interface, not the concretion.
- **Exceptions:** under `Pnlx\Exception\`. Domain-specific ones extend
  `PHPNativeLibraryException`.
- **Static method vs free function:** a stateless helper that **shadows or extends
  a PHP builtin** is a namespaced free function (e.g. `Pnlx\Util\is_null`, used via
  `use function` so it shadows `\is_null`). A cohesive helper that does not shadow
  a builtin is a static method (e.g. `Util::cString`). Do not mix the two for one
  helper.

## 5. Code Generation and Templates

- **All emitted code goes through `.tpl` (Handlebars).** Never `format!` or
  concatenate target-language code in Rust. Shared fragments are partials; each
  generated file shape has one template.
- **The `\{{` trap:** never write a literal backslash immediately before `{{` in a
  template (Handlebars treats `\{{` as an escape). Build any fully-qualified name
  *including its leading `\`* in Rust and pass it into the template as a value.
- **Generated files carry the `!!! DO NOT EDIT THIS FILE !!!` header** from the
  shared template block.
- **Determinism:** emit in sorted order so output is byte-stable for golden tests.

## 6. Schema Conventions

- **Property names are `snake_case`**, without exception. The Rust serde field name
  **equals** the JSON property name — avoid `#[serde(rename)]`; if you reach for it,
  the schema name is probably wrong.
- **Names must disambiguate the axis they describe.** Two fields that both mean
  "things this needs" must say *which* kind — e.g. `native_libraries` (the native C
  libraries a package binds) vs `dependencies` (the pnl packages and co-load
  libraries it pulls in).
- **Versioning:** schemas are OpenAPI 3.0.3 documents under a date directory
  (`schemas/<name>/<date>/schema.json`) and each document carries `schema_version`.
  A breaking shape change bumps the date directory and ships a migration path; the
  loader checks `schema_version`.
- **Both runtimes validate against the same files** (Rust `jsonschema`, PHP runtime
  validator), and the schema files themselves are validated as OpenAPI documents by
  `composer validate:schemas`.

## 7. Generated-Code Contract

The shape of the generated PHP is a **public contract**, frozen at v1.0.0. Changes
to it after the freeze require a `schema_version` bump and a deprecation path. The
golden snapshots under `tests/golden/example/` are the **canonical specification**
of this surface. As of now it is:

| Shape              | Form                                                          |
|--------------------|--------------------------------------------------------------|
| Entity class       | `<Class> extends \Pnlx\Extension\AbstractExtension` in `Pnlx\<Class>` |
| CData base wrapper | `<Class>Context implements \Pnlx\Types\PointerInterface`      |
| Struct types       | `<tag> extends \Pnlx\<Class>\<Class>Context` in `Pnlx\<Class>\Types` |
| Enums              | `enum <tag>: int` in `Pnlx\<Class>\Enums`, with `toInt()` and `->name` (no `__toString` — fatal on PHP enums) |
| Free functions     | `Pnlx\Func\<Class>\<symbol>`                                  |
| Scalar wrappers    | `Pnlx\Types\*` (e.g. `Int_`, `Double`, `String_`)            |
| Exception          | `<Class>Exception extends PHPNativeLibraryException`          |
| Manifest           | `<Class>Manifest implements ManifestInterface`               |
| Attributes         | `AutoGeneratedByPnlx`, `NativeLibrary`, `NativeLibraryComponent`, `RawNativeName`, `StaticInline` under `Pnlx\Attribute\` |

When changing the generator, regenerate the golden snapshots and review the diff;
an unintended change to this table is a contract break.

## 8. Testing Conventions

- **Rust:** unit tests live in-module under `#[cfg(test)]`. Codegen uses `insta`
  snapshots; review with `cargo insta review`. Cross-cutting generated artifacts
  live in `tests/golden/` and regenerate with `UPDATE_GOLDEN=1`.
- **PHP:** PHPUnit, including the golden-artifact comparison.
- **Full sweep on global changes.** Any change to the shared prelude,
  `builtin_type_names`, `all_type_names`, or a shared template is global — run the
  **full isolated pnl-packages sweep** (each package in its own project), never
  just the target package. Precedent: a prelude fix once broke `libgsl` and only
  the sweep caught it.
- **Coverage via multiple examples.** Each package ships several examples
  exercising distinct APIs (not just a version probe); the sweep runs all of them,
  so "it installs" and "it actually works" are both tested.
- **The gate** (run before any commit):

  ```sh
  cargo fmt --all --check \
    && cargo clippy --all-targets --locked -- -D warnings \
    && cargo test --locked \
    && composer analyse && composer cs && composer test
  ```
