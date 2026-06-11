# コマンド

[← ドキュメント目次](../../README.ja.md) · [English](../en/commands.md)

## 目次

- [`pnl` コマンド](#pnl-コマンド)
  - [`pnl help`](#pnl-help)
  - [`pnl version`](#pnl-version)
  - [`pnl -i` / `pnl --information`](#pnl--i--pnl---information)
  - [`pnl -l` / `pnl --license`](#pnl--l--pnl---license)
  - [`pnl init`](#pnl-init)
  - [`pnl install <source>`](#pnl-install-source)
  - [`pnl update [vendor/package]`](#pnl-update-vendorpackage)
  - [`pnl uninstall <vendor/package>`](#pnl-uninstall-vendorpackage)
  - [`pnl list [glob]`](#pnl-list-glob)
  - [`pnl find [glob]`](#pnl-find-glob)
  - [`pnl list native`](#pnl-list-native)
  - [`pnl list repos`](#pnl-list-repos)
  - [`pnl repo add <git|file|https> <url> [--key <key>]`](#pnl-repo-add-gitfilehttps-url---key-key)
  - [`pnl repo index <dir> --base-url <url>`](#pnl-repo-index-dir---base-url-url)
  - [`pnl repo sign <repository-index.json> --key <key>`](#pnl-repo-sign-repository-indexjson---key-key)
  - [`pnl repo remove <url>`](#pnl-repo-remove-url)
  - [`pnl validate`](#pnl-validate)
  - [`pnl self-upgrade`](#pnl-self-upgrade)
  - [起動時のアップデート確認](#起動時のアップデート確認)
  - [`pnl purge cache`](#pnl-purge-cache)
- [`pnlx` コマンド](#pnlx-コマンド)
  - [`pnlx help`](#pnlx-help)
  - [`pnlx version`](#pnlx-version)
  - [`pnlx init`](#pnlx-init)
  - [`pnlx validate`](#pnlx-validate)
  - [`pnlx gen <target> [--library-key <key>]`](#pnlx-gen-target---library-key-key)
  - [`pnlx build [vendor/package ...]`](#pnlx-build-vendorpackage-)
  - [`pnlx publish`](#pnlx-publish)
  - [`pnlx package`](#pnlx-package)

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

### `pnl -i` / `pnl --information`

neofetch 風のバナーを表示します。pnl の AA ロゴの横に、バージョン、OS とアーキテクチャ、ホスト名、バイナリの場所、リポジトリ URL、ライセンス、コピーライト、そして現在のワークスペースにインストール済みの拡張とそのインストール先を表示します。`pnlx -i` / `pnlx --information` は pnlx のロゴで同じ内容を表示します。

```sh
pnl -i
```

出力例です。

```text
  ██████╗ ███╗   ██╗██╗        pnl 0.1.5
  ██╔══██╗████╗  ██║██║        ─────────
  ██████╔╝██╔██╗ ██║██║        OS:         macos (aarch64)
  ██╔═══╝ ██║╚██╗██║██║        Host:       mymachine.local
  ██║     ██║ ╚████║███████╗   Binary:     /usr/local/bin/pnl
  ╚═╝     ╚═╝  ╚═══╝╚══════╝   Repository: https://github.com/m3m0r7/pnl
                               Packages:   https://github.com/m3m0r7/pnl-packages
                               License:    MIT (run `pnl --license` for details)
                               Copyright:  Copyright (c) 2026 memory
                               Extensions: libsdl/libsdl 2.32.10 (./@pnlx/packages/libsdl/libsdl/2.32.10)
```

### `pnl -l` / `pnl --license`

LICENSE ファイルの中身をそのまま表示します（ビルド時にバイナリへ埋め込まれます）。続けて、帰属表示が必要なサードパーティコンポーネントも出力します: ランタイムの Rust クレート、同梱／動的読み込みのネイティブライブラリ（vendored libgit2・OpenSSL、実行時読み込みの libclang）、PHP SDK のランタイム composer パッケージです。`pnlx -l` / `pnlx --license` も同じ内容を表示します。

```sh
pnl --license
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

パッケージ名だけの場合、設定済みの `repositories` を [`priority`](configuration.md#pnljson-の書き方) の高い順に参照し、最後に組み込みの既定リポジトリ `https://github.com/m3m0r7/pnl-packages`（内部的に priority 0 として保持。`pnl.json` には書き込まれません）へフォールバックします。まず `repository-index.json` を参照し、`key` 付きリポジトリでは隣接する `repository-index.json.sig` を Ed25519 で検証します。index から選んだパッケージは `dist.sha256` と実際のパッケージ内容を照合します。`@<version>` でバージョンを固定できます（git は対応するタグ/ブランチを checkout し、解決したバージョンと一致検証）。**ソースを省略**すると、ロックファイルに記録された各拡張をその固定バージョンで復元し、内容を記録済みの sha256 で再検証します。

インストール対象の `pnlx.json` に `dependencies` がある場合は、各依存パッケージを version constraint に合う最新バージョンで先に解決します。既に lock 済みで constraint を満たす依存は再インストールしません。解決結果は lockfile の `dependencies` に記録されます。

パッケージが対象 OS / Linux ディストリビューションの `installation` を宣言している場合、`pnl install` はネイティブライブラリの解決前にそれ（例: `brew install …`）の実行を確認します。パッケージの `checkIfExists` が既に通る場合はスキップします。`-y` / `--yes` でその確認を自動的に許可（`-n` / `--no-interaction` で既定値を採用）できます。Linux ではレシピは `/etc/os-release` から選択されます。ディストリビューションの `ID`（例: `alpine`・`ubuntu`・`fedora`）→ `ID_LIKE` の各祖先（例: `debian`・`rhel`）→ 汎用の `linux` キーの順で照合します。インストールコマンドが失敗した場合は、どのコマンドが失敗したかを表示し、手動でライブラリとヘッダーをインストールしてから改めて `pnl install` を実行するよう案内します。

`installation` または `self_build` を持つパッケージは、`pnlx publish` が `pnlx.json` に記録した `install_script_hash` と現在のコマンド／スクリプト内容を照合します。不一致または未記録の場合、対話時は既定 No で確認します。`-y` 指定時は安全のため停止します。確認を明示的に上書きする場合は `--allow-install-script-hash <sha256>`（複数指定可）を使います。検証なしで通す最後の手段として `--allow-unverified-install-scripts` もあります。

生成される PHP を調整するフラグ:

- `--alias-class <Class>` … 元のクラスを残したまま、`class_alias` で `<Class>` としても参照できるようにします。
- `--function-prefix <prefix>` … 生成されるすべての関数名・メソッド名に `<prefix>` を付けます（接頭辞なしの名前は残しません）。
- `--allow-install-script-hash <sha256>` … 指定した install script hash をこの実行だけ信頼します。複数回指定できます。
- `--allow-unverified-install-scripts` … install script hash の不一致／未記録を許可します。

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

### `pnl list [glob]`

インストール済みの拡張を一覧表示します。`pnl list` は `pnl list extensions` と同じです。glob パターン（`*` と `?`）を渡すと絞り込めます。`vendor/extension` のフル名と末尾（leaf）の両方にマッチするため、`pnl list 'lib*'` は `acme/libusb` も見つけます。

```sh
pnl list
pnl list extensions
pnl list 'lib*'
```

出力例です。

```text
libnfc/libnfc 1.8.0 1.8.0
libsdl/libsdl 2.32.10 2.32.10
libusb/libusb 1.0.29 1.0.29
```

### `pnl find [glob]`

設定済みの `repositories` と組み込みの既定リポジトリから、**インストール可能な**パッケージを一覧表示します（任意で glob 絞り込み）。`pnl list` と同様、パターンはフル名または leaf にマッチします。

各リポジトリが `repository-index.json` を公開していれば、それを取得して軽量に列挙します（GitHub / `https` リポジトリは HTTP 取得、`file` リポジトリはディスクから直読み）。無ければ shallow clone してディレクトリを走査します。同じパッケージを複数のリポジトリが提供する場合は、[`priority`](configuration.md#pnljson-の書き方) が高い方が優先されます（既定リポジトリは最後に参照）。

```sh
# 既定リポジトリの全パッケージを一覧。
pnl find

# 名前が "lib" で始まるものだけ。
pnl find 'lib*'
```

出力例（名前・利用可能バージョン・取得元リポジトリ）です。

```text
libusb/libusb 1.0.29 https://github.com/m3m0r7/pnl-packages/tree/main/packages
libuv/libuv 1.48.0 https://github.com/m3m0r7/pnl-packages/tree/main/packages
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
# ローカルのインデックスフォルダを追加（file:// URL）。
pnl repo add file file:///absolute/path/to/index

# ローカルのインデックスフォルダを追加（プレーンなパス。絶対・プロジェクトからの相対どちらも可）。
pnl repo add file /Users/me/work/pnl-packages
pnl repo add file ./vendor-packages

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

設定したリポジトリは `pnl find` と、名前指定の `pnl install`（bare name 解決）から参照されます。`file` リポジトリは任意の**ローカルディレクトリ**を指せます。`file://` URL でも、プレーンなファイルパス（絶対パス、またはプロジェクトルートからの相対パス）でも構いません。この種のリポジトリは pnl がディスクから直接読み取り、コミット済みの `repository-index.json` があれば優先し、なければツリーを走査してパッケージフォルダを探します。

### `pnl repo index <dir> --base-url <url>`

パッケージのディレクトリから `repository-index.json` を生成します。これにより、`pnl find` がクローンせずにそのリポジトリを一覧できます。`pnlx.json` を含む各パッケージディレクトリを、バージョン・マニフェストパス・コンテンツの `dist.sha256`・`<base-url>/<package-dir>` というインストール可能な `source` URL とともに記録します。

```sh
# リポジトリチェックアウトの packages/ ツリーをインデックス化。
pnl repo index packages \
  --base-url https://github.com/m3m0r7/pnl-packages/tree/main/packages
```

オプション:

- `--output <file>` … インデックスの出力先（既定: `<dir>/repository-index.json`）。
- `--reference <ref>` … 各バージョンに記録する git リファレンス（既定: パッケージのバージョン）。

出力例です。

```text
indexed 106 package(s) into packages/repository-index.json
```

### `pnl repo sign <repository-index.json> --key <key>`

`repository-index.json` に対する detached signature を生成します。署名は `<index>.sig`（既定では `repository-index.json.sig`）に `ed25519:<base64>` 形式で書き込まれます。秘密鍵は 32 bytes の Ed25519 seed を `ed25519:<base64>` または 64 桁 hex で渡します。出力される `repository key` を `pnl repo add ... --key <key>` に設定すると、install/search 時に index 署名が検証されます。

```sh
pnl repo sign packages/repository-index.json --key ed25519:<base64-secret>
pnl repo add https https://example.com/pnl/packages --key ed25519:<base64-public>
```

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

`pnl` / `pnlx` 自身をアップグレードします。https://github.com/m3m0r7/pnl.git からリリースタグを取得し、実行中のバージョンより新しいタグがあれば、そのタグのソースアーカイブをダウンロードして `cargo build --release` でビルドし、バージョン別のレイアウトにインストールします。

```text
~/.local/share/pnl/versions/<version>/bin/pnl
~/.local/share/pnl/versions/<version>/bin/pnlx
~/.local/share/pnl/current -> versions/<version>
/usr/local/bin/pnl  -> ~/.local/share/pnl/current/bin/pnl
/usr/local/bin/pnlx -> ~/.local/share/pnl/current/bin/pnlx
```

`/usr/local/bin` に置かれるのはシンボリックリンクだけなので、アップグレードは `current` リンクの差し替えだけで完了します。バイナリ本体をその場で上書きすることはありません。

```sh
pnl self-upgrade
# /usr/local/bin 以外にシンボリックリンクを置く場合。
pnl self-upgrade --bin-dir ~/bin
# インストール先のルートを変える場合。
pnl self-upgrade --home /opt/pnl
```

すでに最新の場合の出力例です。

```text
pnl 0.1.5 is already the latest release in 0.25s
```

補足:

- インストール先のルートは XDG Base Directory 仕様に従い `$XDG_DATA_HOME/pnl`（デフォルト `~/.local/share/pnl`）です。`--home` または環境変数 `PNL_HOME` で変更できます（`--home` が優先）。
- ビルドには Rust ツールチェイン（`cargo`）が必要です。
- `/usr/local/bin` に書き込めない場合は `sudo` で再実行するか、`--bin-dir` を指定してください。
- プレリリースタグ（例: `v1.0.0-rc.1`）は対象外です。
- `self-upgrade` が扱えるのはバージョン別のシンボリックリンクレイアウトだけです。`pnl` を単体バイナリとして（[リリースページ](https://github.com/m3m0r7/pnl/releases) からダウンロードして `$PATH` に置いて）導入した場合は、その場で差し替えられません。`self-upgrade` は新しいバージョンがあることを知らせ、ダウンロードして再インストールするよう促します。

### 起動時のアップデート確認

`pnl` と `pnlx` は起動時に新しいリリースがないか確認し、あれば 1 行のメッセージを表示します（管理されたインストールなら `pnl self-upgrade`、単体バイナリなら「ダウンロードして再インストール」）。確認結果は `$XDG_CACHE_HOME/pnl` 配下に 1 時間キャッシュされるので、ネットワークへ問い合わせるのは 1 時間に最初の 1 回だけです。メッセージは対話的な端末でのみ表示され、`PNL_NO_UPDATE_CHECK` を設定すると完全に無効化できます。

### `pnl purge cache`

`pnl` が実行間でキャッシュするデータ（ダウンロードしたヘッダー／ライブラリと最新版の確認結果）を削除します。これらはすべて `$XDG_CACHE_HOME/pnl`（デフォルト `~/.cache/pnl`）配下にあります。

```sh
pnl purge cache
```


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

### `pnlx publish`

`pnlx.json` の publish 前メタデータを更新します。現在は `installation` の全コマンド、または `self_build` で指定されたパッケージ相対スクリプトの内容を正規化して sha256 を計算し、`install_script_hash` として `pnlx.json` に書き込みます。

```sh
pnlx publish
```

`self_build` は `installation` と同時には使えません。指定したスクリプトパスはパッケージ相対で、絶対パスや `..` によるトラバーサルは拒否されます。

### `pnlx package`

予約済みのコマンドです。現時点では「未実装」のエラーを返します。
