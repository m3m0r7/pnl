# Installation

[← Documentation index](../../README.md) · [日本語](../ja/installation.md)

## Table of Contents

- [Requirements](#requirements)
- [Install Via Composer](#install-via-composer)
- [Download A Prebuilt Binary](#download-a-prebuilt-binary)
- [Manual Install](#manual-install)

## Requirements

At a minimum, you need the following:

| Tool | Version | What it's for |
| --- | --- | --- |
| PHP CLI | 8.2+ recommended | The `ffi` extension must be loaded and `ffi.enable` must allow CLI FFI. |
| Composer | 2.x | Installs PHPUnit, PHPStan, php-cs-fixer, and `cebe/php-openapi`. |
| Git | 2.x | Needed for Git install sources. |
| Make | any POSIX make | Used by the included `Makefile`. |
| pkg-config | optional but recommended | Used to discover C library versions and include paths. |
| C libraries | per package | The libusb/libnfc/SDL examples need the matching libraries and headers. |

Rust is only needed when you build the `pnl`/`pnlx` binaries from source or work on this repository. Installing and using packages does not compile per-package Rust code.

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

Whether PHP can use FFI is checked at runtime. If PHP cannot use FFI, the SDK raises an FFI-related exception before loading any native library.


## Install Via Composer

In a PHP project, the simplest path is the composer package, which ships the SDK and installs `vendor/bin/pnl` / `vendor/bin/pnlx`. The native binary is built or downloaded on first use (see [PHP Usage](php-usage.md#install-via-composer)):

```sh
composer require m3m0r7/pnl
```

## Download A Prebuilt Binary

If you do not want to build from source, download a prebuilt archive from the [GitHub Releases](https://github.com/m3m0r7/pnl/releases) page. Each tagged release (`v0.1.0`, ...) attaches archives built by GitHub Actions for Linux, macOS, and Windows:

```text
pnl-<version>-x86_64-unknown-linux-gnu.tar.gz
pnl-<version>-x86_64-apple-darwin.tar.gz
pnl-<version>-aarch64-apple-darwin.tar.gz
pnl-<version>-x86_64-pc-windows-msvc.zip
```

Pick the archive for your platform, unpack it, and put `pnl` / `pnlx` somewhere on your `PATH`:

```sh
tar xzf pnl-<version>-aarch64-apple-darwin.tar.gz
sudo install -m 0755 pnl pnlx /usr/local/bin/
```

A binary installed this way is not managed by `pnl self-upgrade` (which only updates the versioned symlink layout below). To update, download the new release and reinstall the same way; `pnl` will tell you when a newer version is available.

## Manual Install

Build the release binaries from source:

```sh
# Build both pnl and pnlx in release mode.
make build
```

This produces `target/release/pnl` and `target/release/pnlx`. Install them:

```sh
# Install under $XDG_DATA_HOME/pnl and link pnl/pnlx from /usr/local/bin.
sudo make install PREFIX=/usr/local
```

`sudo` is needed because the symlinks land in `$PREFIX/bin` (e.g. `/usr/local/bin`). The binaries themselves go into a versioned layout under `$PNL_HOME`, and the `PREFIX` bin directory receives symlinks only:

```text
~/.local/share/pnl/versions/<version>/bin/pnl
~/.local/share/pnl/versions/<version>/bin/pnlx
~/.local/share/pnl/current -> versions/<version>
/usr/local/bin/pnl  -> ~/.local/share/pnl/current/bin/pnl
/usr/local/bin/pnlx -> ~/.local/share/pnl/current/bin/pnlx
```

The install root follows the XDG Base Directory spec: `$XDG_DATA_HOME/pnl`, defaulting to `~/.local/share/pnl`. Change it with `make install PNL_HOME=/path/to/pnl-home`. `pnl self-upgrade` later updates the same layout by switching the `current` link to a newly built version (see [Commands](commands.md)).

During development you can also call `target/debug/pnl` and `target/debug/pnlx` directly after `cargo build`.
