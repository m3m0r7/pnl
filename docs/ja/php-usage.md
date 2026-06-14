# PHP からの使い方

[← ドキュメント目次](../../README.ja.md) · [English](../en/php-usage.md)

## 目次

- [Composer でのインストール](#composer-でのインストール)
- [PHP からの使い方](#php-からの使い方-1)
  - [libusb：バージョン・エラー名・デバイス数を表示する](#libusbバージョンエラー名デバイス数を表示する)
  - [SDL：ウィンドウを開く（オブジェクトのメソッド版）](#sdlウィンドウを開くオブジェクトのメソッド版)
  - [SDL：ウィンドウを開く（グローバル関数版）](#sdlウィンドウを開くグローバル関数版)
- [生成されるファイル](#生成されるファイル)

## Composer でのインストール

SDK と CLI は 1 つの composer パッケージとして導入できます。

```sh
composer require m3m0r7/pnl
```

pnl はただの composer ライブラリです。プラグインではないため composer に `allow-plugins` の許可を求められることはなく、あなたの `composer.json` に手を入れる必要もありません。phpunit が `vendor/bin/phpunit` を入れるのと同じように、`vendor/bin/pnl` / `vendor/bin/pnlx` がインストールされます。

ネイティブバイナリは **CLI を初めて実行したとき** に遅延生成されます。

1. Rust ツールチェイン（https://rustup.rs）があれば同梱ソースからビルドします（プラットフォームに確実に一致）。
2. 無ければ、インストール済みバージョンに対応するリリースのプリビルトバイナリを `https://github.com/m3m0r7/pnl/releases` からダウンロードします。

結果は `vendor/m3m0r7/pnl/bin/.native/<version>/` にキャッシュされるため、2 回目以降はビルドもダウンロードもなくそのまま実行されます。

```sh
vendor/bin/pnl version
vendor/bin/pnl install libusb
```

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

// @pnlx が生成済みパッケージの入口（と一緒に SDK）を読み込みます。
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libusb\Libusb;

// 拡張は自前の runtime を構築します。メタ情報はそのまま参照できます。
$libusb = new Libusb();

// パッケージのメタ情報と、コンパイル済み bridge のパスを取得します。同じ値は
// メソッド呼び出しでも取れます: $libusb->manifest->name(), ->version(), ->path()。
printf("extension: %s %s\n", $libusb->name, $libusb->version);
printf("bridge: %s\n", $libusb->path);
printf("error name for 0: %s\n", $libusb->libusb_error_name(0));
printf("strerror for 0: %s\n", $libusb->libusbStrerror(0));

// 既定のコンテキストで libusb を初期化します。
$result = $libusb->libusbInit(null);
printf("libusb_init: %d (%s)\n", $result, $libusb->libusbErrorName($result));

if ($result === 0) {
    // 生の FFI::new() を使わずに void *[1] を確保します。
    $deviceList = (new \Pnlx\FFI\Allocator())->voidPointerArray(1);

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

// @pnlx が生成済みパッケージの入口（と一緒に SDK）を読み込みます。
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use function Pnlx\Util\is_null;
// SDL_INIT_VIDEO（#define）と SDL_WINDOW_SHOWN（enum 値）はパッケージの const.php に
// 生成されるので、手書きせず import します。
use const Pnlx\Libsdl\SDL_INIT_VIDEO;
use const Pnlx\Libsdl\SDL_WINDOW_SHOWN;

// SDL_WINDOWPOS_CENTERED は関数形式マクロ（SDL_WINDOWPOS_CENTERED_DISPLAY(0)）なので
// 生成されません。手書きで定義します。
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// 生成済みの SDL オブジェクトを `new` するだけ。自前で bridge を読み込みます。
// SDL_Init() や SDL_CreateWindow() などのメソッドを使います。
$sdl = new Libsdl();

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

// @pnlx が生成済みパッケージの入口（と一緒に SDK）を読み込みます。
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
// SDL_INIT_VIDEO（#define）と SDL_WINDOW_SHOWN（enum 値）はパッケージの const.php に
// 生成されるので、手書きせず import します。
use const Pnlx\Libsdl\SDL_INIT_VIDEO;
use const Pnlx\Libsdl\SDL_WINDOW_SHOWN;

// SDL_WINDOWPOS_CENTERED は関数形式マクロ（SDL_WINDOWPOS_CENTERED_DISPLAY(0)）なので
// 生成されません。手書きで定義します。
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

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
