# クイックスタート

[← ドキュメント目次](../../README.ja.md) · [English](../en/quick-start.md)

## クイックスタート

最小の例として、C の `printf` を PHP から呼んでみます。C 標準ライブラリ（`libc`）は macOS・Linux・Windows のどの OS にも最初から入っているため、pnl 以外に**インストールするものは何もありません**。

プロジェクトのディレクトリで:

```sh
# 1. pnl.json を作成。公式パッケージリポジトリは組み込みなので、"repositories"
#    が空でも名前だけで解決できます。
pnl init

# 2. libc パッケージを追加（組み込みの既定リポジトリから解決）。
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

`libc` が最初の題材に向いている理由: `printf`/`puts` は OS に元から入っている C ランタイムの関数なので、`brew install` や `apt-get install` が要りません。パッケージ内でライブラリ項目を `"virtual": true` と宣言しており、これにより pnl は「ディスク上にファイルが無くても名前でリンクする」よう動作します（macOS では libc は dyld 共有キャッシュ内にのみ存在します）。実ライブラリを試す準備ができたら [`pnl install libusb`](commands.md#pnl-install-source) をどうぞ。
