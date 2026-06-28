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
  - [`pnl config <key> [value]`](#pnl-config-key-value)
  - [`pnl compose <members...> --as <Class>`](#pnl-compose-members---as-class)
  - [`pnl update [vendor/package]`](#pnl-update-vendorpackage)
  - [`pnl uninstall <vendor/package>`](#pnl-uninstall-vendorpackage)
  - [`pnl list [glob]`](#pnl-list-glob)
  - [`pnl search [glob]`](#pnl-search-glob)
  - [`pnl info <package>`](#pnl-info-package)
  - [`pnl list native`](#pnl-list-native)
  - [`pnl list repos`](#pnl-list-repos)
  - [`pnl repo add <git|file|https> <url> [--key <key>]`](#pnl-repo-add-gitfilehttps-url---key-key)
  - [`pnl repo index <dir> --base-url <url>`](#pnl-repo-index-dir---base-url-url)
  - [`pnl repo sign <repository-index.json> --key <key>`](#pnl-repo-sign-repository-indexjson---key-key)
  - [`pnl repo remove <url>`](#pnl-repo-remove-url)
  - [`pnl validate`](#pnl-validate)
  - [`pnl doctor`](#pnl-doctor)
  - [`pnl self-upgrade`](#pnl-self-upgrade)
  - [起動時のアップデート確認](#起動時のアップデート確認)
  - [`pnl purge cache`](#pnl-purge-cache)
- [`pnlx` コマンド](#pnlx-コマンド)
  - [`pnlx help`](#pnlx-help)
  - [`pnlx version`](#pnlx-version)
  - [`pnlx init`](#pnlx-init)
  - [`pnlx validate`](#pnlx-validate)
  - [`pnlx gen <target> [--library-key <key>]`](#pnlx-gen-target---library-key-key)
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
0.5.5
```

### `pnl -i` / `pnl --information`

neofetch 風のバナーを表示します。pnl の AA ロゴの横に、バージョン、OS とアーキテクチャ、ホスト名、バイナリの場所、リポジトリ URL、ライセンス、コピーライト、そして現在のワークスペースにインストール済みの拡張とそのインストール先を表示します。`pnlx -i` / `pnlx --information` は pnlx のロゴで同じ内容を表示します。

```sh
pnl -i
```

出力例です。

```text
  ██████╗ ███╗   ██╗██╗        pnl 0.5.5
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
  "library_paths": [],
  // グローバル関数の生成は初期状態でオフ。
  "features": {
    "global_functions": false
  },
  // まだ拡張は入っていません。
  "extensions": {}
}
```

### `pnl install <source>`

指定した拡張をインストールします。具体的には、C ライブラリ本体とヘッダーを探し、PHP ラッパーを生成し、`pnl.json`・ロックファイル・パスマップ・`@pnlx/autoload.php` を更新します。

ソースには URL・ローカルパス・**パッケージ名だけ**・**配布アーカイブ**（`.tar.gz`/`.tgz`/`.zip`。ローカルでもリモートでも可。必要ならダウンロードして展開し、中に `pnlx.json` が無ければエラー）を指定できます。**複数ソースを一度に**渡すこともできます（`pnl install libusb libnfc`）。

パッケージ名だけの場合、設定済みの `repositories` を [`priority`](configuration.md#pnljson-の書き方) の高い順に参照し、最後に組み込みの既定リポジトリ `https://github.com/m3m0r7/pnl-packages`（内部的に priority 0 として保持。`pnl.json` には書き込まれません）へフォールバックします。まず `repository-index.json` を参照し、`key` 付きリポジトリでは隣接する `repository-index.json.sig` を Ed25519 で検証します。index から選んだパッケージは `dist.sha256` と実際のパッケージ内容を照合します。`@<version>` でバージョンを固定できます（git は対応するタグ/ブランチを checkout し、解決したバージョンと一致検証）。**ソースを省略**すると、ロックファイルに記録された各拡張をその固定バージョンで復元し、内容を記録済みの sha256 で再検証します。

インストール対象の `pnlx.json` に `dependencies` がある場合は、各依存パッケージを version constraint に合う最新バージョンで先に解決します。既に lock 済みで constraint を満たす依存は再インストールしません。解決結果は lockfile の `dependencies` に記録されます。

パッケージが対象 OS / Linux ディストリビューションの `setup.install` を宣言している場合、`pnl install` はネイティブライブラリの解決前にそれ（例: `brew install …`）の実行を確認します。パッケージの `check_if_exists` が既に通る場合はスキップします。`-y` / `--yes` でその確認を自動的に許可（`-n` / `--no-interaction` で既定値を採用）できます。Linux ではレシピは `/etc/os-release` から選択されます。ディストリビューションの `ID`（例: `alpine`・`ubuntu`・`fedora`）→ `ID_LIKE` の各祖先（例: `debian`・`rhel`）→ 汎用の `linux` キーの順で照合します。インストールコマンドが失敗した場合は、どのコマンドが失敗したかを表示し、手動でライブラリとヘッダーをインストールしてから改めて `pnl install` を実行するよう案内します。

`setup.install` または `setup.build_script` を持つパッケージは、`pnlx publish` が `pnlx.json` に記録した `setup.build_script_hash` と現在のコマンド／スクリプト内容を照合します。不一致または未記録の場合、対話時は既定 No で確認します。`-y` 指定時は安全のため停止します。確認を明示的に上書きする場合は `--allow-install-script-hash <sha256>`（複数指定可）を使います。検証なしで通す最後の手段として `--allow-unverified-install-scripts` もあります。組み込みの**認可済みリポジトリ**（ファーストパーティの `m3m0r7/pnl-packages` レジストリ。バイナリに埋め込まれた `repositories.authorized` ホワイトリスト参照）からインストールするパッケージは、install スクリプトの実行を信頼してこの確認を省略します。

生成される PHP を調整するフラグ:

- `--alias-class <Class>` … 元のクラスを残したまま、`class_alias` で `<Class>` としても参照できるようにします。
- `--function-prefix <prefix>` … 生成されるすべての関数名・メソッド名に `<prefix>` を付けます（接頭辞なしの名前は残しません）。
- `--enable-use-functions` … `features.global_functions = true` を `pnl.json` に書き込み、生成されるグローバル関数 API を有効にします（[設定](configuration.md#pnljson-の書き方)参照）。
- `--enable-allow-cdata` … `features.cdata_arguments = true` を書き込み、生成シグネチャが生の `\FFI\CData` も受け付けるようにします。
- `--enable-use-php-scalars-in-return` … `features.scalar_returns = true` を書き込み、スカラーに収まる戻り値をネイティブの `int`/`float`/`string` で返すようにします。
- `--enable-use-php-scalars-in-const` … `features.scalar_constants = true` を書き込み、`const.php` が `\Pnlx\Types\*` ラッパーではなくネイティブスカラーを使うようにします（無損失な範囲で）。
- `--enable-static-inline` … `compile_options.static_inline = true` を書き込み、ライブラリの `static inline` 関数を例外スタブではなく呼び出し可能な shim にコンパイルします（C コンパイラが必要 — [設定](configuration.md#static-inline-関数compile_options)参照）。

install スクリプトと整合性に関するフラグ:

- `--allow-install-script-hash <sha256>` … 指定した install script hash をこの実行だけ信頼します。複数回指定できます。
- `--allow-unverified-install-scripts` … install script hash の不一致／未記録を許可します。
- `-f` / `--force` … 解決したコンテンツが lockfile に記録された sha256 と一致しなくても再インストールします。中断せず警告を出し、記録済みダイジェストを新しいコンテンツのもので上書きします。対話実行でフラグ無しの場合、ダイジェスト不一致時に上書き確認を求めます（既定: いいえ）。

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
  ✓ resolved libusb-1.0 1.0.29 libusb-1.0.dylib
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/libusb.ffi.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/Libusb.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbContext.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbException.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbLibraryComponent.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/LibusbManifest.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/const.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/index.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/function.aliases.php
  ✓ generated ./@pnlx/packages/libusb/libusb/1.0.29/src/generated/macro.functions.php
  ✓ installed extension libusb/libusb

added 1 extension in 1.42s
```

生成される主なファイルです。

```text
@pnlx/packages/libusb/libusb/1.0.29/      ← インストールされたパッケージと src/generated
pnlx-lock.json
@pnlx/pnlx-pathmap.json
@pnlx/autoload.php
@pnlx/ide-helper.php                       ← エンティティの IDE／静的解析用スタブ
```

`src/generated` に書き出される全ファイルは [`pnlx gen`](#pnlx-gen-target---library-key-key) を参照してください。

#### コンテンツ整合性（署名）

パッケージのインストール時、`pnl` はコンテンツの署名を計算します。各ファイルを（ソート順で）sha256 化し、それらのハッシュをまとめてもう一度 sha256 にした 1 つのダイジェストを `pnlx-lock.json` の `dist.sha256` に記録します。次回**同じバージョン**をインストールする際は、取得したコンテンツを再びハッシュ化してロック値と比較し、異なる場合は「コンテンツが改ざんされている」としてエラーを出してインストールを中止します。新しいバージョンのインストールは正当な更新として許可します。（生成物・`.git`・ワークスペースディレクトリはダイジェスト対象から除外します。）

### `pnl config <key> [value]`

git config 風に `pnl.json` の設定値を取得・設定します。値を省略すると現在値を表示し、値を指定するとそのキーを設定します。`--unset` でキーを既定値に戻します。

```sh
pnl config compile_options.static_inline          # 現在値を表示
pnl config compile_options.static_inline true     # 設定（true/1/yes/on または false/0/no/off）
pnl config compile_options.static_inline --unset   # 既定値に戻す
```

値は型付きスキーマで検証されるため、未知のキーや真偽値キーへの非真偽値は拒否されます。指定できるキーは `compile_options.static_inline`、`features.*` 各スイッチ、`output_dir` です。

これらの設定は生成物を変えるため、設定変更が成功すると `pnl config` は反映のための再インストールを提案します（対話時のみ。いいえと答えれば後で `pnl install` で反映できます。非対話実行ではリマインダーを表示するだけです）。

### `pnl compose <members...> --as <Class>`

**インストール済み**の拡張を 2 つ以上まとめ、1 つの共有 FFI スコープを通じてすべての関数を公開する単一クラスを生成します。これは `Pnlx\Runtime::compose([...])`（[PHP の使い方](php-usage.md#runtimecompose-パッケージ間で-ffi-スコープを共有する)参照）に対応する、ファイルとして書き出される名前付きの版です。実ファイルとして生成することで、エディタや静的解析が補完できるようになり、合成されたメソッドは（`__call` プロキシではなく）実メソッドなので参照渡しの out パラメータも往復します。

メンバーが 1 つのスコープを共有するため、あるパッケージが生成した `CData`（例: `Sdlimage::IMG_Load` が返す `SDL2_image` の surface）を別のパッケージ（`Libsdl::SDL_CreateTextureFromSurface`）へそのまま渡せます。

```sh
# SDL と SDL_image を 1 つの Pnlx\Sdlx\Sdlx クラスに合成。
pnl compose libsdl sdlimage --as 'Pnlx\Sdlx\Sdlx'
```

引数とオプション:

- `<members...>` … インストール済みのパッケージ名（`vendor/package` または末尾の leaf）を 2 つ以上。
- `--as <Class>` … 生成するクラスの完全修飾名（必須）。
- `--prefix <prefix>` … 2 つのメンバーが同名関数を公開したときの trait メソッド衝突を解決する接頭辞（予約）。

合成結果は `@pnlx/composites/<Class>.php`（`features.global_functions` が有効なら `<Class>Functions.php` も）に書き出し、`pnl.json` に記録し、新しいクラスがメンバーの後に読み込まれるよう `@pnlx/autoload.php` を作り直します。

```text
composed ./@pnlx/composites/Sdlx.php
composed Pnlx\Sdlx\Sdlx from libsdl, sdlimage
```

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
- インストール・生成・パスマップ更新を再実行します。

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

### `pnl search [glob]`

> 別名 `pnl find` でも呼び出せます（同じコマンドです）。

設定済みの `repositories` と組み込みの既定リポジトリから、**インストール可能な**パッケージを一覧表示します（任意で glob 絞り込み）。`pnl list` と同様、パターンはフル名または leaf にマッチします。

各リポジトリが `repository-index.json` を公開していれば、それを取得して軽量に列挙します（GitHub / `https` リポジトリは HTTP 取得、`file` リポジトリはディスクから直読み）。無ければ shallow clone してディレクトリを走査します。同じパッケージを複数のリポジトリが提供する場合は、[`priority`](configuration.md#pnljson-の書き方) が高い方が優先されます（既定リポジトリは最後に参照）。

```sh
# 既定リポジトリの全パッケージを一覧。
pnl search

# 名前が "lib" で始まるものだけ。
pnl search 'lib*'
```

出力例（名前・利用可能バージョン・取得元リポジトリ）です。

```text
libusb/libusb 1.0.29 https://github.com/m3m0r7/pnl-packages/tree/main/packages
libuv/libuv 1.48.0 https://github.com/m3m0r7/pnl-packages/tree/main/packages
```

### `pnl info <package>`

パッケージのリモート情報 — install コマンド、読み込むヘッダー、リンクするネイティブライブラリ — をリポジトリから取得して表示します。ローカルにインストール済みでもリポジトリから取得します。対象にはベース名・`vendor/package`・URL・パスを指定できます。

```sh
# libusb パッケージの情報をインストールせずに表示。
pnl info libusb
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

設定したリポジトリは `pnl search` と、名前指定の `pnl install`（bare name 解決）から参照されます。`file` リポジトリは任意の**ローカルディレクトリ**を指せます。`file://` URL でも、プレーンなファイルパス（絶対パス、またはプロジェクトルートからの相対パス）でも構いません。この種のリポジトリは pnl がディスクから直接読み取り、コミット済みの `repository-index.json` があれば優先し、なければツリーを走査してパッケージフォルダを探します。

### `pnl repo index <dir> --base-url <url>`

パッケージのディレクトリから `repository-index.json` を生成します。これにより、`pnl search` がクローンせずにそのリポジトリを一覧できます。`pnlx.json` を含む各パッケージディレクトリを、バージョン・マニフェストパス・コンテンツの `dist.sha256`・`<base-url>/<package-dir>` というインストール可能な `source` URL とともに記録します。

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

### `pnl doctor`

pnl 拡張のインストール・実行に向けたローカル環境を診断します。

```sh
pnl doctor
```

チェック内容:

- **libclang** — バインディング生成（`pnl install`）に必須。ここの失敗だけが致命的。
- **C コンパイラ** — 任意。`compile_options.static_inline` のシムにのみ必要。
- **pkg-config** — 参考情報。pnl は `.pc` を自前で解析するため、システムの pkg-config は不要。
- **PHP + FFI** — `php` が `PATH` 上にあり、FFI 拡張が読み込まれているか（`ffi.enable` 設定も表示）。
- **ワークスペース** — `pnl.json` の有無とロック済み拡張数。

必須チェックが 1 つでも失敗すると非ゼロ終了します。

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
pnl 0.5.5 is already the latest release in 0.25s
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

`pnlx` は、ライブラリのパッケージを「作る側」のためのツールです。ふだん使うだけなら、直接触る機会はあまりないかもしれません。

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
0.5.5
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

拡張パッケージの `src/generated` 以下に、PHP FFI 定義とラッパーなどを生成します。

```sh
# パッケージ作成用にリポジトリを clone。
git clone https://github.com/m3m0r7/pnl-packages.git

# libusb パッケージのフォルダに移動。
cd pnl-packages/packages/libusb

# FFI 定義・クラス・別名・入口を生成。
pnlx gen libusb
```

生成されるファイルです（`Libusb` はパッケージのクラス名のプレースホルダ）。

```text
src/generated/libusb.ffi.php              # FFI cdef を PHP でラップ
src/generated/Libusb.php                  # エンティティクラス（static ラッパーメソッド）
src/generated/LibusbContext.php           # CData ハンドルのラッパー
src/generated/LibusbException.php         # パッケージ用の生成例外
src/generated/LibusbLibraryComponent.php  # Runtime::compose / pnl compose で使う trait
src/generated/LibusbManifest.php          # インストール時メタデータ（名前・バージョン・パス）
src/generated/const.php                   # 生成された定数
src/generated/index.php                   # 拡張を起動する入口
src/generated/function.aliases.php        # 関数名の別名
src/generated/functions.php               # \Pnlx\Func グローバル関数（global_functions 時）
src/generated/macro.functions.php         # 関数形式マクロを関数として公開
src/generated/types/                      # struct/typedef ラッパーごとに 1 ファイル
src/generated/enums/                      # 名前付き C enum ごとに PHP enum
src/generated/symbol/                     # エクスポートされた C データシンボルのマーカークラス
```

機能依存のバリアントも併せて出力されます。`cdata/`（`cdata_arguments` 時）と `scalar/`（`scalar_returns`/`scalar_constants` 時）。

このコマンドは、

- `pnlx.json` を読み、
- インストール済みプロジェクト内で実行された場合は `@pnlx/pnlx-pathmap.json` からヘッダーを解決し、
- パスマップにヘッダーが無ければ、パッケージの `headers` 設定にフォールバックし、
- PHP のクラス・PHPDoc 付きメソッド・別名・FFI 定義・定数・enum・型ラッパー・入口を生成します。

1 つのパッケージが複数の C ライブラリを必要としていて、ターゲット名だけでは区別できないときは `--library-key` を使います。

```sh
# どの C ライブラリ向けかを明示して生成。
pnlx gen libfoo --library-key libfoo-2.0
```

### `pnlx publish`

`pnlx.json` の publish 前メタデータを更新します。現在は `setup.install` の全コマンド、または `setup.build_script` で指定されたパッケージ相対スクリプトの内容を正規化して sha256 を計算し、`setup.build_script_hash` として `pnlx.json` に書き込みます。

```sh
pnlx publish
```

`setup.build_script` は `setup.install` と同時には使えません。指定したスクリプトパスはパッケージ相対で、絶対パスや `..` によるトラバーサルは拒否されます。

### `pnlx package`

予約済みのコマンドです。現時点では「未実装」のエラーを返します。
