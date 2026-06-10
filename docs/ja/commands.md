# コマンド

[← ドキュメント目次](../../README.ja.md) · [English](../en/commands.md)

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

ソースには URL・ローカルパス・**パッケージ名だけ**・**配布アーカイブ**（`.tar.gz`/`.tgz`/`.zip`。ローカルでもリモートでも可。必要ならダウンロードして展開し、中に `pnlx.json` が無ければエラー）を指定できます。**複数ソースを一度に**渡すこともできます（`pnl install libusb libnfc`）。

パッケージ名だけの場合、設定済みの `repositories` を [`priority`](configuration.md#pnljson-の書き方) の高い順に参照し、最後に組み込みの既定リポジトリ `https://github.com/m3m0r7/pnl-packages`（内部的に priority 0 として保持。`pnl.json` には書き込まれません）へフォールバックします。`@<version>` でバージョンを固定できます（git は対応するタグ/ブランチを checkout し、解決したバージョンと一致検証）。**ソースを省略**すると、ロックファイルに記録された各拡張をその固定バージョンで復元し、内容を記録済みの sha256 で再検証します。

パッケージが対象 OS の `installation` を宣言している場合、`pnl install` はネイティブライブラリの解決前にそれ（例: `brew install …`）の実行を確認します。パッケージの `checkIfExists` が既に通る場合はスキップします。`-y` / `--yes` でその確認を自動的に許可（`-n` / `--no-interaction` で既定値を採用）できます。

生成される PHP を調整するフラグ:

- `--alias-class <Class>` … 元のクラスを残したまま、`class_alias` で `<Class>` としても参照できるようにします。
- `--function-prefix <prefix>` … 生成されるすべての関数名・メソッド名に `<prefix>` を付けます（接頭辞なしの名前は残しません）。

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
pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
```

#### コンテンツ整合性（署名）

パッケージのインストール時、`pnl` はコンテンツの署名を計算します。各ファイルを（ソート順で）sha256 化し、それらのハッシュをまとめてもう一度 sha256 にした 1 つのダイジェストを `pnlx-lock.json` の `dist.sha256` に記録します。次回**同じバージョン**をインストールする際は、取得したコンテンツを再びハッシュ化してロック値と比較し、異なる場合は「コンテンツが改ざんされている」としてエラーを出してインストールを中止します。新しいバージョンのインストールは正当な更新として許可します。（生成物・`.git`・ワークスペースディレクトリはダイジェスト対象から除外します。）

### `pnl update [vendor/package]`

ロックファイルに記録された取得元から、指定した 1 つ、またはインストール済みのすべてを入れ直します。

```sh
# 記録済みの取得元から libusb だけ入れ直します。
pnl update libusb/libusb

# インストール済みの拡張をすべて入れ直します。
pnl update
```

このコマンドは、

- `pnlx-lock.json` を読み、
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

`pnl.json` と、存在すれば `pnlx-lock.json` / `@pnlx/pnlx-pathmap.json` の内容が正しいか検証します。

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

- `pnlx-lock.json` を読み、
- `@pnlx/pnlx-pathmap.json` から C ライブラリのパスを読み、
- インストール済みの `src/generated/*.bridge.rs` を `rustc --crate-type cdylib` でコンパイルし、
- 出来上がったライブラリを `@pnlx/packages/<vendor>/<package>/<version>/bridge/` に書き、
- `@pnlx/pnlx-pathmap.json` の `bridges` を更新します。

### `pnlx package`

予約済みのコマンドです。現時点では「未実装」のエラーを返します。
