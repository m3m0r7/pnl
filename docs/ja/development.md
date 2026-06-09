# 開発

[← ドキュメント目次](../../README.ja.md) · [English](../en/development.md)

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
