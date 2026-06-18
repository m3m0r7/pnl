# Development

[← Documentation index](../../README.md) · [日本語](../ja/development.md)

## Table of Contents

- [Validation And Development](#validation-and-development)
- [Schemas](#schemas)

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

The schema files themselves are checked too: `composer validate:schemas` (part of `composer test`) loads every `schemas/*/*/schema.json` with `cebe/php-openapi` and asserts it is a valid OpenAPI document, so a malformed schema fails CI.
