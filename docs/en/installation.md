# Installation

[← Documentation index](../../README.md) · [日本語](../ja/installation.md)

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


## Install Via Composer

In a PHP project, the simplest path is the composer package, which ships the SDK and builds the CLI binaries into `vendor/bin` on install (see [PHP Usage](php-usage.md#install-via-composer)):

```sh
composer require m3m0r7/pnl
```

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

Install the binaries:

```sh
# Install under $XDG_DATA_HOME/pnl and link pnl/pnlx from /usr/local/bin.
make install PREFIX=/usr/local
```

The binaries are installed into a versioned layout, and the `PREFIX` bin directory receives symlinks only:

```text
~/.local/share/pnl/versions/<version>/bin/pnl
~/.local/share/pnl/versions/<version>/bin/pnlx
~/.local/share/pnl/current -> versions/<version>
/usr/local/bin/pnl  -> ~/.local/share/pnl/current/bin/pnl
/usr/local/bin/pnlx -> ~/.local/share/pnl/current/bin/pnlx
```

The install root follows the XDG Base Directory spec: `$XDG_DATA_HOME/pnl`, defaulting to `~/.local/share/pnl`. It can be changed with `make install PNL_HOME=/path/to/pnl-home`. `pnl self-upgrade` later updates the same layout by switching the `current` link to a newly built version (see [Commands](commands.md)).

During development you can also call `target/debug/pnl` and `target/debug/pnlx` directly after `cargo build`. That said, the examples in this README assume the installed `pnl` / `pnlx` binaries.

GitHub Actions builds release archives for Linux, macOS, and Windows. Every workflow run uploads artifacts, and tag pushes such as `v0.1.0` attach those archives to a GitHub Release.

Example release archive names:

```text
pnl-<version>-x86_64-unknown-linux-gnu.tar.gz
pnl-<version>-x86_64-apple-darwin.tar.gz
pnl-<version>-aarch64-apple-darwin.tar.gz
pnl-<version>-x86_64-pc-windows-msvc.zip
```
