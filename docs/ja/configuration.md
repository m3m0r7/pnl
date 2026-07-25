# 設定

[← ドキュメント目次](../../README.ja.md) · [English](../en/configuration.md)

## 目次

- [プロジェクトの構成](#プロジェクトの構成)
- [`pnl.json` の書き方](#pnljson-の書き方)
  - [ネイティブライブラリの探索](#ネイティブライブラリの探索)

## プロジェクトの構成

拡張をインストールしたあとの PHP プロジェクトは、次のような構成になります。`@pnlx/` 以下は自動生成されるので、基本的に手で編集する必要はありません。

```text
project-root/
  composer.json
  pnl.json                     ← あなたが編集する設定ファイル
  pnlx-lock.json               ← ロックファイル（コミット対象。@pnlx の外）
  @pnlx/                       ← 以下はすべて自動生成
    autoload.php
    ide-helper.php
    pnlx-pathmap.json
    runtime/                    ← Pnlx SDK ランタイムのコピー
    composites/                 ← `pnl compose` で合成したクラス（あれば）
    packages/
      vendor/
        package/
          <version>/            ← インストール済みバージョンごとに1ディレクトリ
            pnlx.json
            src/generated/
```

覚えておくとよいファイルです。

- `pnl.json`: あなたが編集するプロジェクト設定ファイル。
- `pnlx-lock.json`: 入れたバージョンと内容ハッシュを固定するロックファイル。場所が可変な `@pnlx/` の外、`pnl.json` と同じ階層に置かれるので、固定された・コミット可能なパスになります。
- `@pnlx/pnlx-pathmap.json`: 現在の環境向けに生成される、ライブラリ本体・ヘッダーの場所をまとめた地図。
- `@pnlx/autoload.php`: インストール済みパッケージをまとめて読み込むための、自動生成された PHP の入口。
- `@pnlx/ide-helper.php`: 実行時には読み込まれない、IDE／静的解析にエンティティのシグネチャを見せるためのスタブ。
- `@pnlx/runtime/`: `Pnlx` SDK ランタイムのコピー。Composer の autoloader 無しでもオートローダーが動くようにします。
- `@pnlx/composites/`: [`pnl compose`](commands.md#pnl-compose-members---as-class) が生成するクラス。複数の拡張を 1 つの共有 FFI スコープに合成したもの（実行するまでは存在しません）。


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
  "library_paths": [],
  // オプション機能のスイッチ。
  "features": {
    // 生成されるグローバル関数は、初期状態ではオフにしておきます。
    "global_functions": false
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
  "library_paths": [
    "/opt/homebrew/lib",
    "/usr/local/lib"
  ],
  // C 言語風のグローバル関数を生成して使えるようにします。
  "features": {
    "global_functions": true
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
| `library_paths` | 配列 | はい | システム標準や環境変数由来のパスより先に探す、C ライブラリのフォルダ。 |
| `output_dir` | 文字列 | いいえ | 生成物（ロック・パスマップ・インストール済みパッケージ・autoload）の出力先（プロジェクトルートからの相対）。既定は `@pnlx`。 |
| `features.global_functions` | 真偽値 | はい | `true` にすると、C 関数名の PHP 関数を `\Pnlx\Func` 名前空間配下に生成します。`features` オブジェクトを書く場合は必須です（オブジェクト自体は省略可能）。 |
| `features.cdata_arguments` | 真偽値 | いいえ | `true` にすると、生成されるメソッド/関数の引数がラッパー型に加えて生の `\FFI\CData` も受け付けます。手書きの FFI コードと連携するときに便利です。 |
| `features.scalar_params` | 真偽値 | いいえ | `true`（既定）にすると、メソッドが素の PHP スカラー（`int`/`float`/`string`）をそのまま引数に取れます。`false` の場合、スカラーは対応する `\Pnlx\Types\*` 値型でラップして渡す必要があります。 |
| `features.scalar_returns` | 真偽値 | いいえ | `true` にすると、C の戻り値型が PHP スカラーに収まるメソッドは `\Pnlx\Types\*` ラッパーではなくネイティブの `int`/`float`/`string` を返します。 |
| `features.scalar_constants` | 真偽値 | いいえ | `true` にすると、生成される `const.php` が無損失に表現できる定数を `\Pnlx\Types\*` ラッパーではなくネイティブの `int`/`float`/`string` で表現します。 |
| `compile_options.static_inline` | 真偽値 | いいえ | `true` にすると、ライブラリの `static inline` 関数を、例外を投げるスタブではなく呼び出し可能なメソッドにするためのコンパイル済みトランポリン shim をビルドします。インストール時に C コンパイラが必要です（下記参照）。既定は `false`。 |
| `config` | オブジェクト | いいえ | バイナリに埋め込まれた既定エンドポイントのプロジェクト単位の上書き（下記参照）。省略すると既定値を使います。 |
| `extensions` | オブジェクト | はい | 入れたい拡張を `vendor/package` をキーにして並べます。`pnl install` がここに自動で追記します。 |

### static inline 関数（`compile_options`）

C の `static inline` 関数はヘッダー内で完結して定義され、シンボルをエクスポートしないため、PHP FFI からバインドできません。既定では pnl はメソッド自体は生成しますが、呼び出すと `UnsupportedNativeFunctionException` を投げます（メソッドには `#[\Pnlx\Attribute\StaticInline]` が付きます）。

これらの関数が必要なら、オプトインします:

```jsonc
"compile_options": {
  "static_inline": true
}
```

有効にすると、`pnl install` は各 `static inline` 関数の小さな C トランポリンを生成し、パッケージの隣で小さな共有ライブラリにコンパイルして co-load します。これでその関数は通常の呼び出し可能なメソッドになります。pnl が C を再実装することはなく、実際の C コンパイラに処理を任せます。

- **インストール時に C コンパイラが必要です。** pnl は `$CC` → `cc` → `clang` → `gcc` の順に `$PATH` を探索します（`$CFLAGS`/`$LDFLAGS` も尊重）。パッケージに実際に `static inline` 関数があり、かつコンパイラが見つからない場合、インストールは実行可能なメッセージを出して失敗します。これは pnl のインストール要件にコンパイラを追加する唯一の項目なので、厳密にオプトインです（既定のインストールに必要なのは libclang だけ）。
- **グレースフルフォールバック。** コンパイラはあるがパッケージの shim をコンパイルできない場合（例: パーサーが許容して落としたインクルードをヘッダーが必要とする場合）でも、インストールは**失敗しません**。該当関数は例外スタブのまま残り、警告が表示されます。つまりこのオプションを有効にしてもインストールが壊れることはありません。
- これは**利用者側**の設定です（あなたの `pnl.json` に置く）。パッケージ作者が強制することはできないため、インストールするパッケージに勝手にコンパイラ要件が追加されることはありません。

直接書くか、[`pnl config`](commands.md#pnl-config-key-value) / `pnl install --enable-static-inline` で設定できます。

### 既定エンドポイントの上書き

pnl はビルド時に `config.toml` から 2 つの既定エンドポイントを埋め込みます。1 つは名前指定インストールで参照するパッケージレジストリ、もう 1 つは pnl 自身のリリース元リポジトリ（起動時の更新チェックと `pnl self-upgrade` が使用）です。pnl のフォークやプライベートなレジストリを自分でホストする場合は、`config` でプロジェクトごとに上書きできます。

```jsonc
"config": {
  // `pnl self-upgrade` と更新チェックが新しい pnl リリースを探す先。
  "release_repository": "https://github.com/acme/pnl",
  // 名前指定インストールの最も優先度が低いフォールバックレジストリ。
  "package_repository": "https://github.com/acme/pnl-packages/tree/main/packages"
}
```

どちらの項目も任意で、省略した項目は埋め込みの既定値にフォールバックします。

`repositories` に書ける取得元の例です。

```jsonc
// ローカルのパッケージインデックス（file:// URL）。
{ "type": "file", "url": "file://packages" }

// ローカルのパッケージインデックス（プレーンなディレクトリパス。絶対・相対どちらも可）。
{ "type": "file", "url": "/Users/me/work/pnl-packages" }

// Git で管理されたパッケージインデックス。
{ "type": "git", "url": "git@github.com:vendor/pnl-index.git" }

// 署名検証用のキー欄を予約した HTTPS インデックス。
{ "type": "https", "url": "https://example.com/pnl/index.json", "key": "ed25519:<public-key>" }
```

`type` には `file`・`git`・`https` を指定できます。`file` リポジトリは、パッケージを置いた**ローカルディレクトリ**を指します。`file://` URL でも、プレーンなファイルパス（絶対パス、またはプロジェクトルートからの相対パス）でも構いません。`pnl search` とベース名指定の `pnl install` はこのディレクトリをディスクから列挙し、コミット済みの `repository-index.json`（[`pnl repo index`](commands.md#pnl-repo-index-dir---base-url-url) を参照）があれば優先し、なければディレクトリを走査します。`key` は任意で、将来の署名付きインデックス用に予約されています。なお、ローカルパス・`file://` URL・Git URL を `pnl install` に直接渡す場合は、`repositories` の指定は不要です。

`library_paths` は「C ライブラリ本体（.so / .dylib など）」を探すフォルダで、ヘッダー（include）のフォルダではありません。ヘッダーの探索にはライブラリの `.pc` ファイル（直接パースします。`pkg-config` バイナリは不要）、C の include 用環境変数、パッケージ同梱の include、一般的なシステムの include フォルダを使います。

### ネイティブライブラリの探索

既定では `pnl install` は各 C ライブラリをローカル（`library_paths`、`DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`、`PATH`、一般的なシステムフォルダ）から、ヘッダーをライブラリの `.pc` ファイルや include パスから探します。`pnlx.json` の各要件はリモート取得元を指定でき、その場合アセットは一度だけダウンロードしてキャッシュされます。

```jsonc
"native_libraries": {
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

対応スキームは `http`/`https`・`ftp`・`ftps`（TLS 上の FTP）・`git`（リポジトリ内のファイルは `/tree/<branch>/<path>` URL か `ssh`/`git`/`.git` URL で取得）です。リモート取得元が無い要件では従来どおりローカル探索にフォールバックします。

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

バージョン指定は、完全一致・比較範囲・キャレット・チルダに対応します。`*` は任意のセマンティックバージョンを許可します。比較子は `&`（かつ）と `|`（または）で結合でき、`&` が `|` より強く結合します。括弧でグループ化も可能です。例: `>=1.0.0 & <2.0.0`、`>=1.0.0 & <2.0.0 | >=3.0.0`、`(>=1.0.0 & <2.0.0) | >=3.0.0`。`1.2.3` のようにベタ書きすると完全一致になります。`required` は今のところ「依存の意図を表すメモ」で、現状の MVP ではインストール元を明示してインストールします。

グローバル関数モードの例です。

```jsonc
// オプション機能は、トップレベルの "features" に書きます。
"features": {
  // true にすると、C 言語風のグローバル PHP 関数を生成できます。
  "global_functions": true
}
```

有効にすると、生成されたパッケージの入口が、同名の関数がまだ無いときに限り `\Pnlx\Func\Libusb\libusb_init()` のように `\Pnlx\Func\<Class>`（パッケージごとに1セグメント）配下へ関数を定義します。完全修飾で呼ぶか、`use function Pnlx\Func\Libusb\libusb_init;` で読み込んでから `libusb_init()` と呼びます。名前空間に置くことでグローバル名前空間を汚しません。無効の場合は、エンティティクラスの static メソッドとして直接呼び出します（`<Class>::libusb_init()`）。
