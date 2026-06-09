# pnl

[English](README.md)

## 目次

- [pnl とは](#pnl-とは)
- [ざっくりした流れ](#ざっくりした流れ)
- [クイックスタート](#クイックスタート)
- [ステータス](#ステータス)
- [必要なもの](#必要なもの)
- [ビルドとインストール](#ビルドとインストール)
- [プロジェクトの構成](#プロジェクトの構成)
- [`pnl.json` の書き方](#pnljson-の書き方)
- [インストール元の指定方法](#インストール元の指定方法)
- [`pnl` コマンド](#pnl-コマンド)
- [`pnlx` コマンド](#pnlx-コマンド)
- [PHP からの使い方](#php-からの使い方)
- [生成されるファイル](#生成されるファイル)
- [検証と開発](#検証と開発)
- [スキーマ](#スキーマ)
- [制限事項](#制限事項)
- [ライセンス](#ライセンス)

## pnl とは

ひとことで言うと、**PHP から「C 言語で書かれた既存のライブラリ」を簡単に使えるようにするツール**です。

世の中には、USB 機器を操作する `libusb`、NFC を扱う `libnfc`、ウィンドウや画像・音を扱う `SDL` のように、長年使われてきた便利なライブラリがたくさんあります。ただし、これらはほとんどが C 言語向けに作られていて、PHP からそのまま呼び出すのは大変です。

PHP には FFI（Foreign Function Interface）という「他言語のライブラリを直接呼ぶ仕組み」がありますが、自分で使おうとすると、関数の型を手で書き写したり、ポインタを直接さわったりと、かなり骨が折れます。

pnl は、この面倒な部分を自動化します。具体的には次のことをやってくれます。

- ライブラリの「パッケージ」をインストールする
- パソコンにインストール済みの C ライブラリ本体とヘッダー（型情報）を探し出す
- PHP から呼び出すための **ラッパー（PHP のクラスやメソッド）を自動生成**する
- PHP と C の橋渡しをする小さな **bridge（Rust 製のつなぎコード）** を自動でコンパイルする
- 生成したコードを `Pnlx` という PHP SDK 経由で呼び出せるようにする

イメージとしては、Composer がパッケージ管理をしてくれるのと同じ感覚で、C ライブラリを PHP から使えるようにしてくれる、と考えてください。

このリポジトリには 2 つのコマンドラインツールが含まれます。

- **`pnl`**: ライブラリを「使う側」のツール。インストール、ロックファイル管理、検証、一覧表示を行います。
- **`pnlx`**: ライブラリのパッケージを「作る側」のツール。ラッパーの生成や、インストール済み bridge の再ビルドを行います。

ふだんライブラリを使うだけなら、基本的に `pnl` だけ覚えれば十分です。

なお、生成されたインストール内容は Composer の `vendor/` ではなく、専用の `@pnlx/` ディレクトリに置かれます。PHP 側では、まず Composer の autoload で SDK を読み込み、続いて `@pnlx/autoload.php` でインストール済みの拡張を読み込みます。

## ざっくりした流れ

初めて使うときの典型的な流れは次のとおりです。

1. `pnl` と `pnlx` をビルド／インストールする（[ビルドとインストール](#ビルドとインストール)）
2. プロジェクトで `pnl init` を実行し、設定ファイル `pnl.json` を作る
3. `pnl install <ライブラリのURL>` で使いたいライブラリを入れる
4. PHP コードから `Pnlx\Runtime` 経由でライブラリを呼ぶ（[PHP からの使い方](#php-からの使い方)）

まずは雰囲気をつかみたい場合、3 のインストールと 4 のサンプルコードから読むのがおすすめです。

## クイックスタート

最小の例として、C の `printf` を PHP から呼んでみます。C 標準ライブラリ（`libc`）は macOS・Linux・Windows のどの OS にも最初から入っているため、pnl 以外に**インストールするものは何もありません**。

プロジェクトのディレクトリで:

```sh
# 1. pnl.json を作成（既定で公式パッケージリポジトリが入っています）。
pnl init

# 2. libc パッケージを追加。名前だけ指定するとリポジトリから解決されます。
pnl install libc
```

PHP から呼びます（`quickstart.php`）:

```php
<?php

declare(strict_types=1);

require_once __DIR__ . '/vendor/autoload.php';   // Pnlx SDK（Composer 経由）
require_once __DIR__ . '/@pnlx/autoload.php';    // インストール済み拡張

use Pnlx\Libc\Libc;
use Pnlx\Runtime;

$runtime = new Runtime(__DIR__);

/** @var Libc $libc */
$libc = $runtime->load(Libc::class);

$libc->printf("Hello, World from libc!\n");
$libc->puts("And this line is printed by libc puts.");
```

```sh
php quickstart.php
```

```text
Hello, World from libc!
And this line is printed by libc puts.
```

`libc` が最初の題材に向いている理由: `printf`/`puts` は OS に元から入っている C ランタイムの関数なので、`brew install` や `apt-get install` が要りません。パッケージ内でライブラリ項目を `"virtual": true` と宣言しており、これにより pnl は「ディスク上にファイルが無くても名前でリンクする」よう動作します（macOS では libc は dyld 共有キャッシュ内にのみ存在します）。実ライブラリを試す準備ができたら [`pnl install libusb`](#pnl-install-source) をどうぞ。

## ステータス

このリポジトリは初期実装（プロトタイプ）です。

`pnlx.json` を含むパッケージであれば、ローカルのパス・`file://`・Git/GitHub の URL から直接インストールする機能は動作します。一方で、リポジトリインデックスを使った依存解決、署名付きパッケージインデックス、アーカイブのダウンロード、FTP ダウンロードなどはまだ設計段階です。

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

## プロジェクトの構成

拡張をインストールしたあとの PHP プロジェクトは、次のような構成になります。`@pnlx/` 以下は自動生成されるので、基本的に手で編集する必要はありません。

```text
project-root/
  composer.json
  pnl.json                     ← あなたが編集する設定ファイル
  @pnlx/                       ← 以下はすべて自動生成
    autoload.php
    pnlx-lock.json
    pnlx-pathmap.json
    packages/
      vendor/
        package/
          <version>/            ← インストール済みバージョンごとに1ディレクトリ
            pnlx.json
            src/generated/
            bridge/             ← このバージョン用にコンパイルされたネイティブブリッジ
              <package>.bridge.rs
              lib<package>_bridge.dylib
```

覚えておくとよいファイルです。

- `pnl.json`: あなたが編集するプロジェクト設定ファイル。
- `@pnlx/pnlx-lock.json`: 現在の環境向けに生成されるロックファイル（入れたバージョンを固定する記録）。
- `@pnlx/pnlx-pathmap.json`: 現在の環境向けに生成される、ライブラリ本体・ヘッダー・bridge の場所をまとめた地図。
- `@pnlx/autoload.php`: インストール済みパッケージをまとめて読み込むための、自動生成された PHP の入口。

## `pnl.json` の書き方

`pnl.json` は、プロジェクト側の設定ファイルです。次のことを書きます。

- 拡張パッケージをどこから探すか
- C ライブラリ本体をどのフォルダから探すか
- オプション機能を有効にするか
- どの拡張を入れたいか

最小構成は次のとおりです。

```jsonc
{
  // 検証に使うスキーマのバージョン（schemas/pnl/<version>/schema.json を選びます）。
  "schema_version": "2026-07-01",
  // URL から直接インストールするだけなら、リポジトリ指定は空でかまいません。
  "repositories": [],
  // C ライブラリを探す追加のフォルダ。
  "load_paths": [],
  // オプション機能のスイッチ。
  "features": {
    // 生成されるグローバル関数は、初期状態ではオフにしておきます。
    "use_functions": false
  },
  // 入れたい拡張を vendor/package の形で並べます。
  "extensions": {}
}
```

ローカル開発でよく使う例です。

```jsonc
{
  // この pnl.json のスキーマバージョン。
  "schema_version": "2026-07-01",
  // パッケージの取得元（任意）。
  "repositories": [
    {
      // ローカルのファイルリポジトリ。
      "type": "file",
      "url": "file://packages"
    }
  ],
  // システム標準より先に探す、C ライブラリのフォルダ。
  "load_paths": [
    "/opt/homebrew/lib",
    "/usr/local/lib"
  ],
  // C 言語風のグローバル関数を生成して使えるようにします。
  "features": {
    "use_functions": true
  },
  // このプロジェクトが必要とする拡張。
  "extensions": {
    "libusb/libusb": {
      // libusb ラッパーの 1.x 系を許可します。
      "version": ">=1.0.0 & <2.0.0",
      // このプロジェクトに必須の拡張として扱います。
      "required": true
    }
  }
}
```

各項目の意味です。

| 項目 | 型 | 必須 | 意味 |
| --- | --- | --- | --- |
| `schema_version` | 文字列 | はい | 設定ファイルのスキーマバージョン。現在は `2026-07-01`。 |
| `repositories` | 配列 | はい | パッケージの取得元。現状はファイルリポジトリのフォールバック用で、本格的なインデックス解決は未完成です。 |
| `load_paths` | 配列 | はい | システム標準や環境変数由来のパスより先に探す、C ライブラリのフォルダ。 |
| `output_dir` | 文字列 | いいえ | 生成物（ロック・パスマップ・インストール済みパッケージ・autoload）の出力先（プロジェクトルートからの相対）。既定は `@pnlx`。 |
| `features.use_functions` | 真偽値 | はい | `true` にすると、C 関数名の PHP 関数を `\Pnlx\Func` 名前空間配下に生成します。 |
| `extensions` | オブジェクト | はい | 入れたい拡張を `vendor/package` をキーにして並べます。`pnl install` がここに自動で追記します。 |

`repositories` に書ける取得元の例です。

```jsonc
// ローカルのパッケージインデックス。
{ "type": "file", "url": "file://packages" }

// Git で管理されたパッケージインデックス。
{ "type": "git", "url": "git@github.com:vendor/pnl-index.git" }

// 署名検証用のキー欄を予約した HTTPS インデックス。
{ "type": "https", "url": "https://example.com/pnl/index.json", "key": "ed25519:<public-key>" }
```

`type` には `file`・`git`・`https` を指定できます。`key` は任意で、将来の署名付きインデックス用に予約されています。なお、ローカルパス・`file://` URL・Git URL を `pnl install` に直接渡す場合は、`repositories` の指定は不要です。

`load_paths` は「C ライブラリ本体（.so / .dylib など）」を探すフォルダで、ヘッダー（include）のフォルダではありません。ヘッダーの探索には `pkg-config`、C の include 用環境変数、パッケージ同梱の include、一般的なシステムの include フォルダを使います。

### ネイティブライブラリの探索

既定では `pnl install` は各 C ライブラリをローカル（`load_paths`、`DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`、`PATH`、一般的なシステムフォルダ）から、ヘッダーを `pkg-config`/include パスから探します。`pnlx.json` の各要件はリモート取得元を指定でき、その場合アセットは一度だけダウンロードしてキャッシュされます。

```jsonc
"requires": {
  "mylib-1.0": {
    "library_names": ["libmylib.so", "mylib.dll"],
    // $PATH の代わりに http(s) / ftp / git tree URL から取得します。
    "library_url": "https://example.com/releases/libmylib.so",
    // ヘッダーも URL から取得、または…
    "header_url": "https://raw.githubusercontent.com/acme/mylib/v1.0/mylib.h",
    // …ファイルが無ければインラインで埋め込みます。
    "header_inline": "int mylib_add(int a, int b);\nconst char *mylib_version(void);\n",
    "symbol_prefix": "mylib_",
    "version": "^1.0.0",
    "required": true
  }
}
```

対応スキームは `http`/`https`・`ftp`・`git`（リポジトリ内のファイルは `/tree/<branch>/<path>` URL か `ssh`/`git`/`.git` URL で取得）です。`ftps` は未対応のため `https` か `ftp` を使ってください。リモート取得元が無い要件では従来どおりローカル探索にフォールバックします。

拡張のバージョン指定の例です。

```jsonc
// 拡張の指定は、トップレベルの "extensions" に書きます。
"extensions": {
  // キーは vendor/package 形式。
  "vendor/package": {
    // 完全一致・範囲・キャレット（^）・チルダ（~）が使えます。
    "version": "^1.2.3",
    // 必須の拡張として扱います。
    "required": true
  }
}
```

バージョン指定は、完全一致・比較範囲・キャレット・チルダに対応します。比較子は `&`（かつ）と `|`（または）で結合でき、`&` が `|` より強く結合します。括弧でグループ化も可能です。例: `>=1.0.0 & <2.0.0`、`>=1.0.0 & <2.0.0 | >=3.0.0`、`(>=1.0.0 & <2.0.0) | >=3.0.0`。`1.2.3` のようにベタ書きすると完全一致になります。`required` は今のところ「依存の意図を表すメモ」で、現状の MVP ではインストール元を明示してインストールします。

グローバル関数モードの例です。

```jsonc
// オプション機能は、トップレベルの "features" に書きます。
"features": {
  // true にすると、C 言語風のグローバル PHP 関数を生成できます。
  "use_functions": true
}
```

有効にすると、生成されたパッケージの入口が、同名の関数がまだ無いときに限り `\Pnlx\Func\Libusb\libusb_init()` のように `\Pnlx\Func\<Class>`（パッケージごとに1セグメント）配下へ関数を定義します。完全修飾で呼ぶか、`use function Pnlx\Func\Libusb\libusb_init;` で読み込んでから `libusb_init()` と呼びます。名前空間に置くことでグローバル名前空間を汚しません。無効の場合は、`$runtime->load(...)` で取得したオブジェクトのメソッドとして呼び出します。

## インストール元の指定方法

`pnl install` は `packages/<name>` のようなローカルパスだけでなく、いくつかの形式で取得元を指定できます。

現在対応している形式です。

```sh
# ローカルの拡張フォルダからインストール。
pnl install /absolute/path/to/extension-root

# file:// URL からインストール。
pnl install file:///absolute/path/to/extension-root

# GitHub リポジトリ内のパッケージフォルダからインストール。
pnl install https://github.com/m3m0r7/pnl-packages/packages/libusb

# GitHub の tree URL からインストール（"main" が clone するブランチになります）。
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# パッケージのサブフォルダを含む scp 形式の SSH URL からインストール。
pnl install git@github.com:m3m0r7/pnl-packages/packages/libusb
```

GitHub の HTTPS URL と scp 形式の SSH URL では、ホスト名のあとの最初の 2 つのパス（`owner/repository`）をリポジトリとして扱い、残りをそのリポジトリ内のパッケージの場所として扱います。`/tree/<branch>/...` を含む GitHub の URL も使えます。この場合は `<branch>` を clone するブランチ、残りをパッケージの場所として扱います。

たとえば次の URL は、

```text
https://github.com/m3m0r7/pnl-packages/packages/libusb
https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
git@github.com:m3m0r7/pnl-packages/packages/libusb
```

次のリポジトリとして clone され、

```text
https://github.com/m3m0r7/pnl-packages.git
git@github.com:m3m0r7/pnl-packages.git
```

次のパッケージフォルダからインストールされます。

```text
packages/libusb
```

clone した内容は一時的にシステムの一時フォルダに置かれます。Linux なら `/tmp/pnl/git/...`、macOS なら `/var/folders/.../T/pnl/git/...` のような場所です。`@pnlx/packages/<vendor>/<package>/<version>` にコピーされるのは、`pnlx.json` を含む、解決済みのパッケージフォルダだけです。

いずれの形式でも、解決されたローカルのパスに `pnlx.json` が無ければインストールは失敗します。

FTP/FTPS の指定は認識されますが、ダウンロード処理と署名検証が実装されるまでは、はっきりとエラーになります。

## `pnl` コマンド

### `pnl help`

コマンドの概要と、使えるサブコマンドの一覧を表示します。

```sh
pnl help
```

### `pnl version`

`pnl` のバージョンを表示します。

```sh
pnl version
```

出力例です。

```text
0.1.0
```

### `pnl init`

`pnl.json` が無ければ、初期設定ファイルを作ります。

```sh
pnl init
```

出力例です。

```text
initialized ./pnl.json
```

作られる内容です。

```jsonc
{
  // pnl.json 検証用のスキーマバージョン。
  "schema_version": "2026-07-01",
  // 初期状態ではリポジトリ未設定。
  "repositories": [],
  // 初期状態では追加の C ライブラリフォルダ未設定。
  "load_paths": [],
  // グローバル関数の生成は初期状態でオフ。
  "features": {
    "use_functions": false
  },
  // まだ拡張は入っていません。
  "extensions": {}
}
```

### `pnl install <source>`

指定した拡張をインストールします。具体的には、C ライブラリ本体とヘッダーを探し、PHP/Rust のラッパーを生成し、bridge をコンパイルし、`pnl.json`・ロックファイル・パスマップ・`@pnlx/autoload.php` を更新します。

ソースには URL・ローカルパスのほか、**パッケージ名だけ**も指定できます（設定済みの `repositories` から解決。既定の `pnl.json` には公式 `pnl-packages` リポジトリが入っています）。`@<version>` を付けるとバージョンを固定できます（git の場合は対応するタグ/ブランチを checkout し、解決したパッケージのバージョンと一致検証します）。**ソースを省略**して `pnl install` を実行すると、ロックファイルに記録された各拡張をその固定バージョンで復元し、各パッケージの内容を記録済みの sha256 で再検証します。

```sh
# パッケージ名だけで libusb を入れます（既定リポジトリから解決）。
pnl install libusb

# URL から、必要ならバージョン固定で。
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install libusb@1.0.29

# ロックファイルから一括復元（sha256 検証あり）。
pnl install
```

出力例です。

```text
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
  ✓ resolved libusb-1.0 1.0.27 libusb-1.0.dylib
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/libusb.ffi.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/Libusb.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/LibusbContext.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/index.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/functions.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/function.aliases.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.27/src/generated/libusb.bridge.rs
  ✓ installed extension libusb/libusb

added 1 extension in 1.42s
```

生成される主なファイルです。

```text
@pnlx/packages/libusb/libusb/1.0.27/
@pnlx/packages/libusb/libusb/1.0.27/bridge/libusb_bridge.dylib
@pnlx/pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
```

#### コンテンツ整合性（署名）

パッケージのインストール時、`pnl` はコンテンツの署名を計算します。各ファイルを（ソート順で）sha256 化し、それらのハッシュをまとめてもう一度 sha256 にした 1 つのダイジェストを `@pnlx/pnlx-lock.json` の `dist.sha256` に記録します。次回**同じバージョン**をインストールする際は、取得したコンテンツを再びハッシュ化してロック値と比較し、異なる場合は「コンテンツが改ざんされている」としてエラーを出してインストールを中止します。新しいバージョンのインストールは正当な更新として許可します。（生成物・`.git`・ワークスペースディレクトリはダイジェスト対象から除外します。）

### `pnl update [vendor/package]`

ロックファイルに記録された取得元から、指定した 1 つ、またはインストール済みのすべてを入れ直します。

```sh
# 記録済みの取得元から libusb だけ入れ直します。
pnl update libusb/libusb

# インストール済みの拡張をすべて入れ直します。
pnl update
```

このコマンドは、

- `@pnlx/pnlx-lock.json` を読み、
- 記録された `source.url` を再利用し、
- インストール・生成・bridge のコンパイル・パスマップ更新を再実行します。

### `pnl uninstall <vendor/package>`

拡張を `pnl.json` とロックから削除し、インストール済みのフォルダを消し、`@pnlx/autoload.php` を作り直します。

```sh
# libusb を pnl.json・ロック／パスマップ・@pnlx/packages から削除します。
pnl uninstall libusb/libusb
```

出力例です。

```text
uninstalled libusb/libusb
```

### `pnl list`

インストール済みの拡張を一覧表示します。`pnl list` は `pnl list extensions` と同じです。

```sh
pnl list
pnl list extensions
```

出力例です。

```text
libnfc/libnfc 1.8.0 1.8.0
libsdl/libsdl 2.32.10 2.32.10
libusb/libusb 1.0.29 1.0.29
```

### `pnl list native`

`@pnlx/pnlx-pathmap.json` をもとに、見つかった C ライブラリ本体を一覧表示します。

```sh
pnl list native
```

出力例です。

```text
libnfc 1.8.0 /opt/homebrew/lib/libnfc.dylib
libusb-1.0 1.0.29 /opt/homebrew/lib/libusb-1.0.dylib
sdl2 2.32.10 /opt/homebrew/lib/libSDL2.dylib
```

### `pnl list repos`

`pnl.json` に設定したリポジトリを一覧表示します。

```sh
# まずローカルのファイルリポジトリを追加。
pnl repo add file file:///tmp/pnl-packages-demo

# 設定済みのリポジトリを表示。
pnl list repos
```

出力例です。

```text
File file://packages
File file:///tmp/pnl-packages-demo
```

### `pnl repo add <git|file|https> <url> [--key <key>]`

`pnl.json` に拡張インデックスのリポジトリを追加します。同じ URL は重複して追加されません。

```sh
# ローカルのインデックスフォルダを追加。
pnl repo add file file:///absolute/path/to/index

# Git のインデックスリポジトリを追加。
pnl repo add git git@github.com:vendor/pnl-index.git

# HTTPS インデックスと、将来の署名チェック用の公開鍵を追加。
pnl repo add https https://example.com/pnl/index.json --key ed25519:<public-key>
```

結果です。

```jsonc
{
  // リポジトリの種類。
  "type": "file",
  // pnl.json に保存される URL。
  "url": "file:///absolute/path/to/index"
}
```

なお、リポジトリインデックスの解決はまだ未完成です。このコマンドは、今後の解決処理と、ローカルファイルリポジトリのフォールバック探索のために設定を記録しておくものです。

### `pnl repo remove <url>`

`pnl.json` からリポジトリを削除します。

```sh
pnl repo remove file:///absolute/path/to/index
```

成功時は何も出力しません。

### `pnl validate`

`pnl.json` と、存在すれば `@pnlx/pnlx-lock.json` / `@pnlx/pnlx-pathmap.json` の内容が正しいか検証します。

```sh
pnl validate
```

出力例です。

```text
pnl workspace is valid
```

検証する内容です。

- `schema_version` に基づく OpenAPI スキーマ検証
- パッケージ名と SemVer（バージョン表記）のチェック
- ロック／パスマップの環境一致チェック
- パスマップ／ロックの整合性チェック

### `pnl self-upgrade`

予約済みのコマンドです。現時点では「未実装」のエラーを返します。

## `pnlx` コマンド

`pnlx` は、ライブラリのパッケージを「作る側」のためのツールです。ふだん使うだけなら、`pnlx build` 以外はあまり触らないかもしれません。

### `pnlx help`

コマンドの概要とサブコマンドの一覧を表示します。

```sh
pnlx help
```

### `pnlx version`

`pnlx` のバージョンを表示します。

```sh
pnlx version
```

出力例です。

```text
0.1.0
```

### `pnlx init`

拡張パッケージのフォルダに `pnlx.json` が無ければ、初期ファイルを作ります。

```sh
# 新しいパッケージ用のフォルダを作成。
mkdir -p packages/example

# そのフォルダに移動。
cd packages/example

# pnlx.json が無ければ作成。
pnlx init
```

出力例です。

```text
initialized ./pnlx.json
```

### `pnlx validate`

拡張パッケージの `pnlx.json` を検証します。

```sh
# パッケージ作成用にリポジトリを clone。
git clone https://github.com/m3m0r7/pnl-packages.git

# libusb パッケージのフォルダに移動。
cd pnl-packages/packages/libusb

# pnlx.json とパッケージ固有の値を検証。
pnlx validate
```

出力例です。

```text
pnlx workspace is valid
```

### `pnlx gen <target> [--library-key <key>]`

拡張パッケージの `src/generated` 以下に、PHP/Rust のラッパーなどを生成します。

```sh
# パッケージ作成用にリポジトリを clone。
git clone https://github.com/m3m0r7/pnl-packages.git

# libusb パッケージのフォルダに移動。
cd pnl-packages/packages/libusb

# FFI 定義・クラス・別名・入口・bridge の Rust を生成。
pnlx gen libusb
```

生成されるファイルです。

```text
src/generated/libusb.ffi.php
src/generated/Libusb.php
src/generated/LibusbContext.php
src/generated/index.php
src/generated/function.aliases.php
src/generated/libusb.bridge.rs
```

このコマンドは、

- `pnlx.json` を読み、
- インストール済みプロジェクト内で実行された場合は `@pnlx/pnlx-pathmap.json` からヘッダーを解決し、
- パスマップにヘッダーが無ければ、パッケージの `headers` 設定にフォールバックし、
- PHP のクラス・PHPDoc 付きメソッド・別名・FFI 定義・入口・Rust bridge を生成します。

1 つのパッケージが複数の C ライブラリを必要としていて、ターゲット名だけでは区別できないときは `--library-key` を使います。

```sh
# どの C ライブラリ向けかを明示して生成。
pnlx gen libfoo --library-key libfoo-2.0
```

### `pnlx build [vendor/package ...]`

インストール済みパッケージの、コンパイル済み Rust bridge を再ビルドします。

```sh
# インストール済みの bridge をすべてビルド。
pnlx build

# 名前が一意なら、末尾の名前だけでビルド可能。
pnlx build libusb

# 複数まとめてビルド。
pnlx build libusb libnfc libsdl

# vendor/package のフルネームでもビルド可能。
pnlx build libusb/libusb
```

出力例です。

```text
built 3 bridge(s)
```

このコマンドは、

- `@pnlx/pnlx-lock.json` を読み、
- `@pnlx/pnlx-pathmap.json` から C ライブラリのパスを読み、
- インストール済みの `src/generated/*.bridge.rs` を `rustc --crate-type cdylib` でコンパイルし、
- 出来上がったライブラリを `@pnlx/packages/<vendor>/<package>/<version>/bridge/` に書き、
- `@pnlx/pnlx-pathmap.json` の `bridges` を更新します。

### `pnlx package`

予約済みのコマンドです。現時点では「未実装」のエラーを返します。

## PHP からの使い方

まず、サンプル用のパッケージをインストールします。

```sh
# libusb・libnfc・SDL のラッパーをインストール。
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libnfc
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libsdl

# インストール後に bridge を再ビルド。
pnlx build
```

### libusb：バージョン・エラー名・デバイス数を表示する

libusb のバージョンやエラー名、つながっているデバイス数を表示する例です。

```php
<?php

declare(strict_types=1);

// 呼び出し元のカレントディレクトリに関係なく、プロジェクト直下で動かします。
chdir(__DIR__);

// Composer が SDK を、@pnlx が生成済みパッケージの入口を読み込みます。
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libusb\Libusb;
use Pnlx\Runtime;

// Runtime が設定・パスマップ・生成済み入口・bridge をまとめて解決します。
$runtime = new Runtime(__DIR__);

/** @var Libusb $libusb */
// 生成済みの libusb オブジェクトを Runtime 経由で取得します。
$libusb = $runtime->load(Libusb::class);

// パッケージのメタ情報と、コンパイル済み bridge のパスを取得します。
$context = $runtime->context(Libusb::class);

printf("extension: %s %s\n", $context->name(), $context->version());
printf("bridge: %s\n", $context->path());
printf("error name for 0: %s\n", $libusb->libusb_error_name(0));
printf("strerror for 0: %s\n", $libusb->libusbStrerror(0));

// 既定のコンテキストで libusb を初期化します。
$result = $libusb->libusbInit(null);
printf("libusb_init: %d (%s)\n", $result, $libusb->libusbErrorName($result));

if ($result === 0) {
    // 生の FFI::new() を使わずに void *[1] を確保します。
    $deviceList = $runtime->allocator()->voidPointerArray(1);

    // libusb がデバイス一覧のポインタを $deviceList[0] に書き込みます。
    $deviceCount = $libusb->libusbGetDeviceList(null, $deviceList);

    if ($deviceCount < 0) {
        // 負の値は libusb のエラーコードです。
        printf("device count: failed (%s)\n", $libusb->libusbErrorName($deviceCount));
    } else {
        printf("device count: %d\n", $deviceCount);

        // libusb_get_device_list() が返したデバイス一覧を解放します。
        $libusb->libusbFreeDeviceList($deviceList[0], 1);
    }

    // 既定の libusb コンテキストを終了します。
    $libusb->libusbExit(null);
    echo "libusb_exit: ok\n";
}
```

出力例です。

```text
extension: libusb/libusb 1.0.29
bridge: /path/to/project/@pnlx/packages/libusb/libusb/1.0.27/bridge/libusb_bridge.dylib
error name for 0: LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED
strerror for 0: Success
libusb_init: 0 (LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED)
device count: 6
libusb_exit: ok
```

### SDL：ウィンドウを開く（オブジェクトのメソッド版）

生成されたオブジェクトのメソッドを使って、SDL のウィンドウを開き、その中に "Hello World!" を描画する例です。

```php
<?php

declare(strict_types=1);

// 呼び出し元のカレントディレクトリに関係なく、プロジェクト直下で動かします。
chdir(__DIR__);

// Composer が SDK を、@pnlx が生成済みパッケージの入口を読み込みます。
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use Pnlx\Runtime;
use function Pnlx\Util\is_null;

// SDL のビデオサブシステムを表すフラグ。
const SDL_INIT_VIDEO = 0x00000020;

// ウィンドウを画面中央に置くよう SDL に依頼するための値。
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// 表示状態のウィンドウを作るためのフラグ。
const SDL_WINDOW_SHOWN = 0x00000004;

// Runtime が生成済みの SDL オブジェクトと bridge を読み込みます。
$runtime = new Runtime(__DIR__);

/** @var Libsdl $sdl */
// SDL_Init() や SDL_CreateWindow() などのメソッドを使います。
$sdl = $runtime->load(Libsdl::class);

// "Hello World!" に使う文字用の小さな 5x7 ビットマップフォント。
// '1' が点灯ピクセル、行は上から下です。
$font = [
    'H' => ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'e' => ['00000', '00000', '01110', '10001', '11111', '10000', '01110'],
    'l' => ['01100', '00100', '00100', '00100', '00100', '00100', '01110'],
    'o' => ['00000', '00000', '01110', '10001', '10001', '10001', '01110'],
    'W' => ['10001', '10001', '10001', '10101', '10101', '11011', '10001'],
    'r' => ['00000', '00000', '10110', '11001', '10000', '10000', '10000'],
    'd' => ['00001', '00001', '01101', '10011', '10001', '10001', '01111'],
    '!' => ['00100', '00100', '00100', '00100', '00100', '00000', '00100'],
    ' ' => ['00000', '00000', '00000', '00000', '00000', '00000', '00000'],
];

// 後片付けから参照できるよう、ハンドルは try の外で宣言します。
$window = null;
$renderer = null;
$initialized = false;

try {
    // SDL のビデオサブシステムを起動します。
    $result = $sdl->SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . $sdl->SDL_GetError());
    }
    $initialized = true;

    // ウィンドウと、それに描画するためのレンダラを作ります。
    $window = $sdl->SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() が生の FFI::isNull() チェックを隠してくれます。
        throw new RuntimeException('SDL_CreateWindow failed: ' . $sdl->SDL_GetError());
    }

    $renderer = $sdl->SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . $sdl->SDL_GetError());
    }

    // 背景を暗い色でクリアします。
    $sdl->SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    $sdl->SDL_RenderClear($renderer);

    // 各フォントピクセルをブロックに拡大して、ウィンドウ内に "Hello World!" を描きます。
    // SDL_RenderDrawPoint は整数のみを取るため、FFI 構造体は不要です。
    $sdl->SDL_SetRenderDrawColor($renderer, 0xFF, 0xFF, 0xFF, 0xFF);
    $scale = 6;
    $x = 70;
    $y = 150;
    foreach (str_split('Hello World!') as $char) {
        $glyph = $font[$char] ?? $font[' '];
        foreach ($glyph as $row => $bits) {
            for ($col = 0; $col < 5; $col++) {
                if ($bits[$col] !== '1') {
                    continue;
                }
                for ($dy = 0; $dy < $scale; $dy++) {
                    for ($dx = 0; $dx < $scale; $dx++) {
                        $sdl->SDL_RenderDrawPoint($renderer, $x + $col * $scale + $dx, $y + $row * $scale + $dy);
                    }
                }
            }
        }
        $x += 6 * $scale; // グリフ 5px + 余白 1px
    }

    // フレームを表示し、イベントを処理しながら少しの間ウィンドウを表示し続けます。
    $sdl->SDL_RenderPresent($renderer);
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        $sdl->SDL_PumpEvents();
        $sdl->SDL_Delay(16);
    }
} finally {
    if (!is_null($renderer)) {
        $sdl->SDL_DestroyRenderer($renderer);
    }
    // ウィンドウの作成に成功していれば破棄します。
    if (!is_null($window)) {
        $sdl->SDL_DestroyWindow($window);
    }

    // 初期化に成功していた場合だけ SDL を終了します。
    if ($initialized) {
        $sdl->SDL_Quit();
    }
}
```

### SDL：ウィンドウを開く（グローバル関数版）

生成されたグローバル関数を使って、SDL のウィンドウを開き、その中に "Hello World!" を描画する例です。この書き方を使うには、先に `pnl.json` の `features.use_functions` を `true` にしておく必要があります。

```php
<?php

declare(strict_types=1);

// 呼び出し元のカレントディレクトリに関係なく、プロジェクト直下で動かします。
chdir(__DIR__);

// Composer が SDK を、@pnlx が生成済みパッケージの入口を読み込みます。
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use function Pnlx\Util\is_null;

// 生成されるグローバル関数は \Pnlx\Func 名前空間配下にあります。ここで使うものを
// import して、下の短い名前で呼べるようにします。
use function Pnlx\Func\Libsdl\{
    SDL_Init,
    SDL_GetError,
    SDL_CreateWindow,
    SDL_CreateRenderer,
    SDL_SetRenderDrawColor,
    SDL_RenderClear,
    SDL_RenderDrawPoint,
    SDL_RenderPresent,
    SDL_PumpEvents,
    SDL_Delay,
    SDL_DestroyRenderer,
    SDL_DestroyWindow,
    SDL_Quit,
};

// SDL のビデオサブシステムを表すフラグ。
const SDL_INIT_VIDEO = 0x00000020;

// ウィンドウを画面中央に置くよう SDL に依頼するための値。
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// 表示状態のウィンドウを作るためのフラグ。
const SDL_WINDOW_SHOWN = 0x00000004;

if (!function_exists('Pnlx\\Func\\Libsdl\\SDL_Init')) {
    // @pnlx/autoload.php は features.use_functions が true のときだけ \Pnlx\Func 関数を定義します。
    throw new RuntimeException('SDL global functions are disabled. Set pnl.json features.use_functions to true.');
}

// "Hello World!" に使う文字用の小さな 5x7 ビットマップフォント。
// '1' が点灯ピクセル、行は上から下です。
$font = [
    'H' => ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'e' => ['00000', '00000', '01110', '10001', '11111', '10000', '01110'],
    'l' => ['01100', '00100', '00100', '00100', '00100', '00100', '01110'],
    'o' => ['00000', '00000', '01110', '10001', '10001', '10001', '01110'],
    'W' => ['10001', '10001', '10001', '10101', '10101', '11011', '10001'],
    'r' => ['00000', '00000', '10110', '11001', '10000', '10000', '10000'],
    'd' => ['00001', '00001', '01101', '10011', '10001', '10001', '01111'],
    '!' => ['00100', '00100', '00100', '00100', '00100', '00000', '00100'],
    ' ' => ['00000', '00000', '00000', '00000', '00000', '00000', '00000'],
];

// 後片付けから参照できるよう、ハンドルは try の外で宣言します。
$window = null;
$renderer = null;
$initialized = false;

try {
    // グローバル関数で SDL のビデオサブシステムを起動します。
    $result = SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . SDL_GetError());
    }
    $initialized = true;

    // ウィンドウと、それに描画するためのレンダラを作ります。
    $window = SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() が生の FFI::isNull() チェックを隠してくれます。
        throw new RuntimeException('SDL_CreateWindow failed: ' . SDL_GetError());
    }

    $renderer = SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . SDL_GetError());
    }

    // 背景を暗い色でクリアします。
    SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    SDL_RenderClear($renderer);

    // 各フォントピクセルをブロックに拡大して、ウィンドウ内に "Hello World!" を描きます。
    // SDL_RenderDrawPoint は整数のみを取るため、FFI 構造体は不要です。
    SDL_SetRenderDrawColor($renderer, 0xFF, 0xFF, 0xFF, 0xFF);
    $scale = 6;
    $x = 70;
    $y = 150;
    foreach (str_split('Hello World!') as $char) {
        $glyph = $font[$char] ?? $font[' '];
        foreach ($glyph as $row => $bits) {
            for ($col = 0; $col < 5; $col++) {
                if ($bits[$col] !== '1') {
                    continue;
                }
                for ($dy = 0; $dy < $scale; $dy++) {
                    for ($dx = 0; $dx < $scale; $dx++) {
                        SDL_RenderDrawPoint($renderer, $x + $col * $scale + $dx, $y + $row * $scale + $dy);
                    }
                }
            }
        }
        $x += 6 * $scale; // グリフ 5px + 余白 1px
    }

    // フレームを表示し、イベントを処理しながら少しの間ウィンドウを表示し続けます。
    SDL_RenderPresent($renderer);
    $until = microtime(true) + 3.0;
    while (microtime(true) < $until) {
        SDL_PumpEvents();
        SDL_Delay(16);
    }
} finally {
    if (!is_null($renderer)) {
        SDL_DestroyRenderer($renderer);
    }
    // ウィンドウの作成に成功していれば破棄します。
    if (!is_null($window)) {
        SDL_DestroyWindow($window);
    }

    // 初期化に成功していた場合だけ SDL を終了します。
    if ($initialized) {
        SDL_Quit();
    }
}
```

## 生成されるファイル

生成された PHP/Rust ファイルの先頭には、次の情報を含むヘッダーコメントが付きます。

- 生成日時
- 生成したホスト名
- 生成時の OS／アーキテクチャ
- PHP のバージョン

生成されたパッケージのファイルは、再生成のたびに上書きされる可能性があります。手を加えたい処理は、`src/generated` を直接いじるのではなく、`src/` 側に上書き（オーバーライド）として追加してください。

## 検証と開発

手早く確認するためのチェックです。

```sh
# Rust のフォーマットを確認。
cargo fmt --check

# Rust のテストを実行。
cargo test

# PHPUnit のテストを実行。
composer test

# php-cs-fixer をチェックモードで実行。
composer cs

# PHPStan を実行。
composer analyse

# プロジェクトの設定と生成済みの状態を検証。
pnl validate
```

インストールがひととおり動くか確認するスモークテストです。

```sh
# リポジトリから libusb をインストール。
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# インストール後にプロジェクトを検証。
pnl validate

# インストール済みの libusb bridge を再ビルド。
pnlx build libusb

# 「PHP からの使い方」の libusb サンプルを実行。
php <libusb-example.php>
```

生成された PHP の構文チェックです。

```sh
# @pnlx/packages 以下の生成済み PHP ファイルを構文チェック。
find @pnlx/packages -name '*.php' -print0 | xargs -0 -n1 php -l
```

## スキーマ

各 JSON ファイルの形式は `schema_version` でバージョン管理されています。現在のスキーマは OpenAPI 3.0.3 のドキュメントとして、次の場所にあります。

```text
schemas/pnl/2026-07-01/schema.json
schemas/pnlx/2026-07-01/schema.json
schemas/pnlx-lock/2026-07-01/schema.json
schemas/pnlx-pathmap/2026-07-01/schema.json
schemas/repository-index/2026-07-01/schema.json
```

Rust の CLI と PHP の SDK は、独自の検証を行う前に、これらのスキーマで検証します。

## 制限事項

- リポジトリインデックスの解決はまだ未完成です。
- FTP/FTPS のインストール元は認識されますが、ダウンロードはされません。
- アーカイブ配布物のダウンロードと展開は未実装です。
- リポジトリインデックスの署名は未実装です。
- マクロや static inline な C 関数は、リンク可能な bridge 関数として表現されない限り、PHP からは使えません。
- ロック／パスマップは 1 つの環境専用です。環境が一致しない場合、検証と実行時の読み込みでエラーになります。

## ライセンス

このリポジトリは `composer.json` 上では MIT です。同梱される C ライブラリは、それぞれ元の（upstream の）ライセンスを保持します。詳しくは `https://github.com/m3m0r7/pnl-packages` のパッケージマニフェストと README を確認してください。
