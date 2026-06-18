# インストール

[← ドキュメント目次](../../README.ja.md) · [English](../en/installation.md)

## 目次

- [必要なもの](#必要なもの)
- [Composer でのインストール](#composer-でのインストール)
- [ビルド済みバイナリのダウンロード](#ビルド済みバイナリのダウンロード)
- [手動インストール](#手動インストール)

## 必要なもの

最低限、次のものがあれば使えます。

| ツール | 必要なバージョン | 用途 |
| --- | --- | --- |
| PHP CLI | 8.2 以上を推奨 | `ffi` 拡張が有効で、`ffi.enable` が CLI での FFI を許可している必要があります。 |
| Composer | 2.x | PHPUnit、PHPStan、php-cs-fixer、`cebe/php-openapi` を導入します。 |
| Git | 2.x | Git からインストールする場合に必要です。 |
| Make | POSIX 互換の make | 同梱の `Makefile` で使います。 |
| pkg-config | 任意（あると便利） | C ライブラリのバージョンや include パスの探索に使います。 |
| C ライブラリ本体 | ライブラリごと | libusb / libnfc / SDL の例には、対応するライブラリとヘッダーが必要です。 |

Rust が必要なのは、`pnl` / `pnlx` バイナリをソースからビルドする場合、またはこのリポジトリ自体を開発・テストする場合だけです。パッケージのインストールと利用では、パッケージごとの Rust コードはコンパイルしません。

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

PHP の FFI が使えるかどうかは実行時にチェックされます。もし PHP が FFI を使えない設定だと、ネイティブライブラリを読み込む前に SDK が FFI 関連の例外を投げます。


## Composer でのインストール

PHP プロジェクトでは composer パッケージが最も簡単です。SDK が入り、`vendor/bin/pnl` / `vendor/bin/pnlx` がインストールされます。ネイティブバイナリは初回実行時にビルド（無ければダウンロード）されます（[PHP からの使い方](php-usage.md#composer-でのインストール) を参照）。

```sh
composer require m3m0r7/pnl
```

## ビルド済みバイナリのダウンロード

ソースからビルドしたくない場合は、[GitHub Releases](https://github.com/m3m0r7/pnl/releases) からビルド済みアーカイブをダウンロードできます。`v0.1.0` のようなタグを付けたリリースごとに、GitHub Actions が Linux・macOS・Windows 向けにビルドしたアーカイブが添付されます。

```text
pnl-<version>-x86_64-unknown-linux-gnu.tar.gz
pnl-<version>-x86_64-apple-darwin.tar.gz
pnl-<version>-aarch64-apple-darwin.tar.gz
pnl-<version>-x86_64-pc-windows-msvc.zip
```

自分のプラットフォーム向けのアーカイブを選んで展開し、`pnl` / `pnlx` を `PATH` の通ったディレクトリに置きます。

```sh
tar xzf pnl-<version>-aarch64-apple-darwin.tar.gz
sudo install -m 0755 pnl pnlx /usr/local/bin/
```

この方法で入れたバイナリは `pnl self-upgrade`（下記のバージョン別シンボリックリンクレイアウトのみを更新します）の管理対象外です。更新するときは新しいリリースをダウンロードして同じ手順で入れ直してください。新しいバージョンがあれば `pnl` が知らせてくれます。

## 手動インストール

ソースからリリース用のバイナリをビルドします。

```sh
# pnl と pnlx の両方をリリースモードでビルドします。
make build
```

`target/release/pnl` と `target/release/pnlx` ができます。これをインストールします。

```sh
# $XDG_DATA_HOME/pnl 配下にインストールし、/usr/local/bin から pnl/pnlx へのシンボリックリンクを張ります。
sudo make install PREFIX=/usr/local
```

シンボリックリンクを `$PREFIX/bin`（例: `/usr/local/bin`）に張るため `sudo` が必要です。バイナリ本体は `$PNL_HOME` 配下のバージョン別レイアウトに置かれ、`PREFIX` の bin ディレクトリにはシンボリックリンクだけが置かれます。

```text
~/.local/share/pnl/versions/<version>/bin/pnl
~/.local/share/pnl/versions/<version>/bin/pnlx
~/.local/share/pnl/current -> versions/<version>
/usr/local/bin/pnl  -> ~/.local/share/pnl/current/bin/pnl
/usr/local/bin/pnlx -> ~/.local/share/pnl/current/bin/pnlx
```

インストール先のルートは XDG Base Directory 仕様に従い `$XDG_DATA_HOME/pnl`（デフォルト `~/.local/share/pnl`）です。`make install PNL_HOME=/path/to/pnl-home` で変更できます。`pnl self-upgrade` も同じレイアウトを使い、新しいバージョンをビルドして `current` リンクを差し替えます（[コマンド](commands.md) を参照）。

開発中は `cargo build` のあと `target/debug/pnl` と `target/debug/pnlx` を直接呼び出すこともできます。
