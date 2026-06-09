# インストール

[← ドキュメント目次](../../README.ja.md) · [English](../en/installation.md)

## 必要なもの

最低限、次のものがあれば使えます。

| ツール | 必要なバージョン | 用途 |
| --- | --- | --- |
| Rust | 1.85 以上を推奨 | Rust 2024 edition を使用。bridge のコンパイル（`rustc`）にも使います。 |
| PHP CLI | 8.2 以上を推奨 | `ffi` 拡張が有効で、`ffi.enable` が CLI での FFI を許可している必要があります。 |
| Composer | 2.x | PHPUnit、PHPStan、php-cs-fixer、`cebe/php-openapi` を導入します。 |
| Git | 2.x | Git からインストールする場合に必要です。 |
| Make | POSIX 互換の make | 同梱の `Makefile` で使います。 |
| pkg-config | 任意（あると便利） | C ライブラリのバージョンや include パスの探索に使います。 |
| C ライブラリ本体 | ライブラリごと | libusb / libnfc / SDL の例には、対応するライブラリとヘッダーが必要です。 |

現在動作確認しているローカル環境は次のとおりです。

```text
rustc 1.92.0
cargo 1.92.0
PHP 8.5.2 CLI NTS
Composer 2.8.8
git 2.52.0
pkg-config 2.5.1
macOS/aarch64（Homebrew で libusb, libnfc, SDL2 を導入）
```

PHP の FFI が使えるかどうかは実行時にチェックされます。もし PHP が FFI を使えない設定だと、bridge を読み込む前に SDK が FFI 関連の例外を投げます。


## ビルドとインストール

リリース用のバイナリをビルドします。

```sh
# pnl と pnlx の両方をリリースモードでビルドします。
make build
```

成功するとこうなります。

```text
cargo build --release --bins
target/release/pnl
target/release/pnlx
```

ビルドしたバイナリを任意の場所にインストールします。

```sh
# リリースバイナリを /usr/local/bin にコピーします。
make install PREFIX=/usr/local
```

成功するとこうなります。

```text
install -d "/usr/local/bin"
install -m 0755 target/release/pnl "/usr/local/bin/pnl"
install -m 0755 target/release/pnlx "/usr/local/bin/pnlx"
```

開発中は `cargo build` のあと `target/debug/pnl` と `target/debug/pnlx` を直接呼び出すこともできます。ただしこの README の例は、基本的にインストール済みの `pnl` / `pnlx` を前提に書いています。

GitHub Actions では Linux・macOS・Windows 向けのリリースアーカイブをビルドします。各ワークフロー実行で成果物（artifact）をアップロードし、`v0.1.0` のようなタグを push すると、それらが GitHub Release に添付されます。

リリースアーカイブの名前の例です。

```text
pnl-<version>-x86_64-unknown-linux-gnu.tar.gz
pnl-<version>-x86_64-apple-darwin.tar.gz
pnl-<version>-aarch64-apple-darwin.tar.gz
pnl-<version>-x86_64-pc-windows-msvc.zip
```
