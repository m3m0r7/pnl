# 規約 (Conventions)

[← ドキュメント目次](../../README.ja.md) · [English](../en/conventions.md)

これは **pnl のソースツリー**（Rust CLI・PHP SDK・テンプレート・JSON スキーマ）の
命名規則とコーディングルールです。ここに書かれているのは *あるべき状態* であり、
現状コードが逸脱している箇所はリファクタリングの対象であって反例ではありません。
v0.7.0 で予定しているリファクタリングはこの文書に従います。

## 目次

- [1. 設計原則](#1-設計原則)
- [2. モジュール構成（アーキ層）](#2-モジュール構成アーキ層)
- [3. Rust の命名・スタイル・lint](#3-rust-の命名スタイルlint)
- [4. PHP SDK の命名・スタイル](#4-php-sdk-の命名スタイル)
- [5. コード生成とテンプレート](#5-コード生成とテンプレート)
- [6. スキーマ規約](#6-スキーマ規約)
- [7. 生成コードの契約](#7-生成コードの契約)
- [8. テスト規約](#8-テスト規約)

## 1. 設計原則

以下のすべてに優先します。スタイル上のルールがこれらを破る理由にはなりません。

- **本質的対応のみ。** 失敗している個別ライブラリではなく、一般的な問題を解く。
  ジェネレータ内での**ライブラリ個別分岐は禁止**、**stopgap 禁止**、その依存なしでは
  本当に解けない場合を除き**新規依存の追加は禁止**。
- **No-drop（関数を落とさない）。** 関数を黙って捨てない。束縛できない場合
  （シンボルを持たない static inline、表現不能な型）でもメソッド／関数は出力し、
  内部で **throw** して該当属性を付ける。FFI の `cdef` と PHP のメソッド面は分離して
  いるので、`cdef` エントリのないスタブはロードを壊さない。
- **スキーマが source of truth。** すべての設定の形は JSON スキーマで定義する。
  Rust CLI と PHP SDK は双方ともそれに従い、独自フィールドを発明しない。
- **連結ではなく生成。** 出力する PHP と C `cdef` はすべて Handlebars テンプレート
  (`.tpl`) を通す。Rust 側で `format!`／文字列連結によって対象言語のコードを組み立てない。
- **決定的な出力。** 安定した（ソート済みの）順序で反復し、生成ファイルをバイト単位で
  安定させる。これが golden／スナップショットテストを意味あるものにする。
- **早く・大きく失敗する。** 黙ったフォールバックより actionable なエラーを優先する。
  可能なものは preflight する（ツールチェイン検査）。カバレッジに上限やスキップを
  設けるなら**明示する** — 黙った打ち切りは「全部やった」と誤読される。

## 2. モジュール構成（アーキ層）

クレートは**層**で構成する。import は**一方向のみ**に流れ、下位層は上位層を import
しない。`cli` だけが他層をオーケストレーションする。

```
util  ←  model  ←  { native, sources }  ←  codegen  ←  cli
                                                         embed（バイナリ境界）
```

| 層           | 責務                                       | モジュール（移動先）                                                                  |
|--------------|--------------------------------------------|--------------------------------------------------------------------------------------|
| `util/`      | ドメイン非依存のヘルパ                     | `glob`, `io`                                                                          |
| `model/`     | データ型・検証・埋め込みスキーマ・設定     | `manifest`, `schema`, `validate`, `version`, `platform`, `config`, `workspace`, `repository_index` |
| `native/`    | C 世界の探索と相互運用                     | `header_adapter`（分割。下記参照）, `pkg_config`, `native`, `cc`, `tbd`, `shim`, `install_script` |
| `sources/`   | パッケージ・アセットの取得                 | `fetch`, `archive`, `cache`, `git_source`                                            |
| `codegen/`   | PHP と `cdef` の出力                        | `generate`（+ `types`, `php`, `names`, `aliases`）, `templates`                       |
| `cli/`       | ユーザー向けコマンドと表示                 | `commands/*`, `ui`, `interaction`, `about`, `highlight`, `self_upgrade`, `release`   |
| `embed/`     | cdylib の C ABI と埋め込み SDK ペイロード  | `ffi`, `sdk_assets`                                                                  |

ルール:

- **1 モジュール 1 目的**。先頭のモジュール doc コメント（`//!`）に明記する。全モジュール必須。
- **god-file を分割する。** ~600 行超のモジュールはリファクタ対象。モジュールが
  ディレクトリに育ったら、`mod.rs` は re-export と配線のみ、ロジックは名前付き
  サブモジュールに置く。`header_adapter.rs`（libclang パース・マクロ展開・const 評価）は
  `native/header_adapter/` ツリー（例: `parse`, `macros`, `consts`, `types`）に分割。
  `install.rs`・`generate.rs` も同様に分解する。
- **libclang・pkg-config ファイル・C ツールチェインに触れてよいのは `native` 層だけ。**
  その上位層は C の内部事情を知らない。
- 新規コードは便利な場所ではなく責務の合う層に置く。どの層にも合わないなら層構成が
  間違っている — catch-all を足す前に相談する。

## 3. Rust の命名・スタイル・lint

- **フォーマット:** `cargo fmt` のデフォルト。特定設定が必要でコメントで正当化できる
  場合を除き `rustfmt.toml` は置かない。
- **lint は CI だけでなくツリー内に。** deny レベルを `Cargo.toml` に成文化する:

  ```toml
  [lints.rust]
  warnings = "deny"

  [lints.clippy]
  all = "deny"
  ```

  これでローカルの `cargo build` が CI と同じ強制をかける。CI 側の `-D warnings` は
  バックストップとして残す。
- **命名:** モジュール・関数は `snake_case`、型／トレイトは `CamelCase`、定数は
  `SCREAMING_SNAKE_CASE`。モジュール名は暗号的略称ではなく**記述的な語**にする。
  外部の確立した名前は、それが**ドメイン用語そのもの**であるときだけ許容する
  （`tbd` = `.tbd` スタブ形式、`cc` = `CC` 規約）。それ以外は綴る。
- **エラー:** 境界では `anyhow::Result` に `.context(...)` で actionable なメッセージを
  付ける。呼び出し側が variant で分岐する必要があるときだけ型付きエラー（`thiserror`）を
  導入する — 現状その必要はないので `anyhow` を維持。
- **テスト以外で `unwrap`／`expect` を使わない**。無謬性が証明できる場合のみ、理由を
  コメントに書いて使う。
- **clone より借用。** `clippy::needless_clone` 等に従う。上記 deny レベルでエラーになる。

## 4. PHP SDK の命名・スタイル

- **スタイル（`.php-cs-fixer.php` に成文化）:** `@PSR12`, `declare(strict_types=1)`,
  `single_quote`, `ordered_imports`, `no_unused_imports`。
- **静的解析:** PHPStan `level: max`、baseline なし。新規コードはクリーンに通す。
- **名前空間:** すべて `Pnlx\` 配下。1 ファイル 1 クラス／インターフェース。
- **インターフェース:** `<Name>Interface` と命名。サービスクラスは具象ではなく
  インターフェースに依存する。
- **例外:** `Pnlx\Exception\` 配下。ドメイン固有のものは `PHPNativeLibraryException` を継承。
- **静的メソッド vs 自由関数:** PHP の組み込みを**シャドウ／拡張する**ステートレスな
  ヘルパは名前空間付き自由関数（例: `Pnlx\Util\is_null`、`use function` で `\is_null` を
  シャドウ）。組み込みをシャドウしない凝集したヘルパは静的メソッド（例: `Util::cString`）。
  1 つのヘルパで両者を混在させない。

## 5. コード生成とテンプレート

- **出力コードはすべて `.tpl`（Handlebars）を通す。** Rust 内で対象言語コードを
  `format!`／連結しない。共有断片は partial に、生成ファイルの各形に 1 テンプレート。
- **`\{{` の罠:** テンプレートで `{{` の直前にバックスラッシュを書かない
  （Handlebars は `\{{` をエスケープ扱いする）。**先頭の `\` を含む FQCN は Rust 側で
  組み立て**、値としてテンプレートに渡す。
- **生成ファイルには共有テンプレートの `!!! DO NOT EDIT THIS FILE !!!` ヘッダ**を付ける。
- **決定性:** ソート順で出力し、golden テスト用にバイト安定にする。

## 6. スキーマ規約

- **プロパティ名は例外なく `snake_case`。** Rust serde のフィールド名は JSON の
  プロパティ名と**一致**させる — `#[serde(rename)]` は避ける。使いたくなったら、たぶん
  スキーマ名の方が間違っている。
- **名前は記述する軸を区別できること。** ともに「これが必要とするもの」を意味する
  2 つのフィールドは、*どの種類*かを名前で示すこと。例: `native_libraries`（パッケージが
  束縛するネイティブ C ライブラリ）vs `dependencies`（co-load する追加ライブラリと pnl パッケージ）。
- **版管理:** スキーマは日付ディレクトリ配下の OpenAPI 3.0.3 文書
  （`schemas/<name>/<date>/schema.json`）で、各文書は `schema_version` を持つ。破壊的な
  形状変更は日付ディレクトリを上げ、マイグレーション経路を用意する。ローダは
  `schema_version` を検査する。
- **両ランタイムが同じファイルで検証する**（Rust `jsonschema`、PHP ランタイム検証器）。
  スキーマファイル自体も `composer validate:schemas` で OpenAPI 文書として検証される。

## 7. 生成コードの契約

生成される PHP の形は **public な契約**であり、v1.0.0 で凍結する。凍結後の変更は
`schema_version` の更新と deprecation 経路を要する。`tests/golden/example/` の golden
スナップショットがこの面の**正本仕様**である。現状は以下:

| 形                  | 形式                                                          |
|---------------------|--------------------------------------------------------------|
| エンティティクラス  | `Pnlx\<Class>` の `<Class> extends \Pnlx\Extension\AbstractExtension` |
| CData 基底ラッパー  | `<Class>Context implements \Pnlx\Types\PointerInterface`      |
| 構造体型            | `Pnlx\<Class>\Types` の `<tag> extends \Pnlx\<Class>\<Class>Context` |
| 列挙型              | `Pnlx\<Class>\Enums` の `enum <tag>: int`、`toInt()` と `->name` 付き（`__toString` は PHP enum で fatal なので不可） |
| 自由関数            | `Pnlx\Func\<Class>\<symbol>`                                 |
| スカラーラッパー    | `Pnlx\Types\*`（例: `Int_`, `Double`, `String_`）           |
| 例外                | `<Class>Exception extends PHPNativeLibraryException`          |
| マニフェスト        | `<Class>Manifest implements ManifestInterface`               |
| 属性                | `Pnlx\Attribute\` 配下の `AutoGeneratedByPnlx`, `NativeLibrary`, `NativeLibraryComponent`, `RawNativeName`, `StaticInline` |

ジェネレータを変更したら golden スナップショットを再生成して diff をレビューする。
この表への意図しない変更は契約破壊である。

## 8. テスト規約

- **Rust:** ユニットテストは `#[cfg(test)]` でモジュール内に置く。コード生成は `insta`
  スナップショット（`cargo insta review` で確認）。横断的な生成物は `tests/golden/` に
  置き、`UPDATE_GOLDEN=1` で再生成する。
- **PHP:** PHPUnit（golden 生成物比較を含む）。
- **グローバル変更時はフル sweep。** 共有 prelude・`builtin_type_names`・`all_type_names`・
  共有テンプレートへの変更はグローバルなので、対象パッケージだけでなく**フル isolated
  pnl-packages sweep**（各パッケージを独立プロジェクトで）を回す。先例: prelude の修正が
  かつて `libgsl` を壊し、sweep だけが検出した。
- **複数 examples によるカバレッジ。** 各パッケージは異なる API を行使する examples を
  複数同梱する（バージョン取得だけにしない）。sweep はそのすべてを実行するので、
  「インストールできる」と「実際に動く」の両方が試される。
- **ゲート**（コミット前に実行）:

  ```sh
  cargo fmt --all --check \
    && cargo clippy --all-targets --locked -- -D warnings \
    && cargo test --locked \
    && composer analyse && composer cs && composer test
  ```
