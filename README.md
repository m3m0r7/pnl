# pnl

[日本語版](README.ja.md)

**pnl makes it easy to use C libraries from PHP.** It installs library "packages", finds the C library and headers already on your machine, generates PHP wrappers plus a small Rust bridge, compiles the bridge, and exposes everything through the `Pnlx` PHP SDK — think Composer, but for C libraries.

```sh
pnl init
pnl install libc
```

See the [Quick Start](docs/en/quick-start.md) to call C `printf` from PHP in a minute.

## Documentation

- [Overview](docs/en/overview.md) — What pnl is, how it works, and project status.
- [Quick Start](docs/en/quick-start.md) — Call C `printf` from PHP in a few commands.
- [Installation](docs/en/installation.md) — Requirements and building/installing the binaries.
- [Configuration](docs/en/configuration.md) — Project layout and writing `pnl.json`.
- [Install Sources](docs/en/install-sources.md) — URLs, paths, bare names, archives, and native discovery.
- [Commands](docs/en/commands.md) — `pnl` and `pnlx` command reference.
- [PHP Usage](docs/en/php-usage.md) — Loading extensions and the generated files.
- [Development](docs/en/development.md) — Validation, testing, and the JSON schemas.

The default package repository is **https://github.com/m3m0r7/pnl-packages**.

## License

This repository is currently marked as MIT in `composer.json`. The bundled C libraries keep their own upstream licenses; see the package manifests and READMEs at `https://github.com/m3m0r7/pnl-packages`.
