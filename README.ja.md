# pnl

[English](README.md)

**pnl は、PHP から「C 言語で書かれた既存のライブラリ」を簡単に使えるようにするツールです。** ライブラリの「パッケージ」をインストールし、マシン上の C ライブラリ本体とヘッダーを探し、PHP のラッパーを生成して、`Pnlx` PHP SDK 経由で呼べるようにします。Composer の C ライブラリ版だと考えてください。

```sh
pnl init
pnl search 'lib*'    # 利用可能なパッケージを一覧（既定リポジトリ＋自分のリポジトリ）
pnl install libc
pnl list 'lib*'      # インストール済みを確認
```

1 分で C の `printf` を PHP から呼ぶには [クイックスタート](docs/ja/quick-start.md) をどうぞ。

## 目次

- [ドキュメント](#ドキュメント)
- [ライセンス](#ライセンス)

## ドキュメント

- [概要](docs/ja/overview.md) — pnl とは何か、仕組み、ステータス。
- [クイックスタート](docs/ja/quick-start.md) — 数コマンドで C の `printf` を PHP から呼ぶ。
- [インストール](docs/ja/installation.md) — 必要なものとバイナリのビルド／インストール。
- [設定](docs/ja/configuration.md) — プロジェクト構成と `pnl.json` の書き方。
- [インストール元](docs/ja/install-sources.md) — URL・パス・名前・アーカイブとネイティブ探索。
- [コマンド](docs/ja/commands.md) — `pnl` と `pnlx` のコマンドリファレンス。
- [PHP からの使い方](docs/ja/php-usage.md) — 拡張の読み込みと生成されるファイル。
- [開発](docs/ja/development.md) — 検証・テスト・JSON スキーマ。
- [規約](docs/ja/conventions.md) — ソースツリーの命名規則・コーディングルール・アーキ層。

公式の既定パッケージリポジトリは **https://github.com/m3m0r7/pnl-packages** です。`repository-index.json` を公開しているため、`pnl search` はクローンせずに一覧できます。リポジトリは短いエイリアス（例: `sdl` → `libsdl`）も公開でき、`pnl install sdl` は参照先のパッケージへ解決されます。組み込みのエンドポイントはプロジェクトごとに上書きできます — [設定](docs/ja/configuration.md) を参照してください。

生成される PHP SDK は専用のオートローダ（`@pnlx/autoload.php`）で自分自身を読み込むため、実行時に Composer のオートローダを必要としません。

## ライセンス

このリポジトリは `composer.json` 上では MIT です。同梱される C ライブラリは、それぞれ元の（upstream の）ライセンスを保持します。詳しくは `https://github.com/m3m0r7/pnl-packages` のパッケージマニフェストと README を確認してください。
