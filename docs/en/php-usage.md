# PHP Usage

[← Documentation index](../../README.md) · [日本語](../ja/php-usage.md)

## Table of Contents

- [Install Via Composer](#install-via-composer)
- [PHP Usage](#php-usage-1)
  - [libusb: version, error name, and device count](#libusb-version-error-name-and-device-count)
  - [SDL: open a window (object methods)](#sdl-open-a-window-object-methods)
  - [SDL: open a window (global functions)](#sdl-open-a-window-global-functions)
- [Generated Files](#generated-files)

## Install Via Composer

The SDK and the CLI come from a single composer package:

```sh
composer require m3m0r7/pnl
```

pnl is an ordinary composer library — no plugin, so composer never asks you to
add it to `allow-plugins`, and your `composer.json` stays untouched. It installs
`vendor/bin/pnl` and `vendor/bin/pnlx` like any other tool (the way phpunit
ships `vendor/bin/phpunit`).

The native binary is produced lazily the **first time you run the CLI**:

1. If a Rust toolchain (https://rustup.rs) is available, the bundled sources are
   built from source — a guaranteed match for your platform.
2. Otherwise pnl downloads the prebuilt binary for the release that matches the
   installed version from `https://github.com/m3m0r7/pnl/releases`.

The result is cached under `vendor/m3m0r7/pnl/bin/.native/<version>/`, so later
runs exec it directly with no build or download.

```sh
vendor/bin/pnl version
vendor/bin/pnl install libusb
```

## PHP Usage

First, install the sample packages:

```sh
# Install the libusb, libnfc, and SDL wrappers.
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libnfc
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libsdl

# Rebuild the bridges after installing.
pnlx build
```

### libusb: version, error name, and device count

An example that prints libusb's version, an error name, and how many devices are connected:

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libusb\Libusb;
use Pnlx\Runtime;

// Runtime resolves the config, pathmap, generated entrypoints, and bridge FFI.
$runtime = new Runtime(__DIR__);

/** @var Libusb $libusb */
// Get the generated libusb object through Runtime.
$libusb = $runtime->load(Libusb::class);

// Read package metadata and the compiled bridge path.
$context = $runtime->context(Libusb::class);

printf("extension: %s %s\n", $context->name(), $context->version());
printf("bridge: %s\n", $context->path());
printf("error name for 0: %s\n", $libusb->libusb_error_name(0));
printf("strerror for 0: %s\n", $libusb->libusbStrerror(0));

// Initialize libusb with the default context.
$result = $libusb->libusbInit(null);
printf("libusb_init: %d (%s)\n", $result, $libusb->libusbErrorName($result));

if ($result === 0) {
    // Allocate void *[1] without exposing raw FFI::new() to user code.
    $deviceList = $runtime->allocator()->voidPointerArray(1);

    // libusb writes the device-list pointer into $deviceList[0].
    $deviceCount = $libusb->libusbGetDeviceList(null, $deviceList);

    if ($deviceCount < 0) {
        // Negative values are libusb error codes.
        printf("device count: failed (%s)\n", $libusb->libusbErrorName($deviceCount));
    } else {
        printf("device count: %d\n", $deviceCount);

        // Release the device list returned by libusb_get_device_list().
        $libusb->libusbFreeDeviceList($deviceList[0], 1);
    }

    // Shut down the default libusb context.
    $libusb->libusbExit(null);
    echo "libusb_exit: ok\n";
}
```

Example output:

```text
extension: libusb/libusb 1.0.29
bridge: /path/to/project/@pnlx/packages/libusb/libusb/1.0.27/bridge/libusb_bridge.dylib
error name for 0: LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED
strerror for 0: Success
libusb_init: 0 (LIBUSB_SUCCESS / LIBUSB_TRANSFER_COMPLETED)
device count: 6
libusb_exit: ok
```

### SDL: open a window (object methods)

Opening an SDL window and drawing "Hello World!" inside it using the generated object's methods:

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libsdl\Libsdl;
use Pnlx\Runtime;
use function Pnlx\Util\is_null;

// Flag for SDL's video subsystem.
const SDL_INIT_VIDEO = 0x00000020;

// Value that asks SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Flag for creating a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

// Runtime loads the generated SDL object and its bridge.
$runtime = new Runtime(__DIR__);

/** @var Libsdl $sdl */
// Use methods like SDL_Init() and SDL_CreateWindow().
$sdl = $runtime->load(Libsdl::class);

// A tiny 5x7 bitmap font for the characters in "Hello World!".
// '1' marks a lit pixel; rows are top-to-bottom.
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

// Declare handles outside try so cleanup can see them.
$window = null;
$renderer = null;
$initialized = false;

try {
    // Start SDL's video subsystem.
    $result = $sdl->SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . $sdl->SDL_GetError());
    }
    $initialized = true;

    // Create a window and a renderer to draw into it.
    $window = $sdl->SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . $sdl->SDL_GetError());
    }

    $renderer = $sdl->SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . $sdl->SDL_GetError());
    }

    // Clear to a dark background.
    $sdl->SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    $sdl->SDL_RenderClear($renderer);

    // Draw "Hello World!" in the window, scaling each font pixel into a block.
    // SDL_RenderDrawPoint takes only integers, so no FFI structs are needed.
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
        $x += 6 * $scale; // 5px glyph + 1px gap
    }

    // Present the frame and keep the window up briefly while pumping events.
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
    // Destroy the window if creation succeeded.
    if (!is_null($window)) {
        $sdl->SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        $sdl->SDL_Quit();
    }
}
```

### SDL: open a window (global functions)

Opening an SDL window and drawing "Hello World!" inside it using the generated global functions. To use this style, first set `features.use_functions` to `true` in `pnl.json`.

```php
<?php

declare(strict_types=1);

// Run from the project root regardless of the caller's working directory.
chdir(__DIR__);

// Composer loads the SDK; @pnlx loads the generated package entrypoints.
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/@pnlx/autoload.php';

use function Pnlx\Util\is_null;

// Generated global functions live under \Pnlx\Func; import the ones used here
// so the short names below resolve.
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

// Flag for SDL's video subsystem.
const SDL_INIT_VIDEO = 0x00000020;

// Value that asks SDL to center the window on the current display.
const SDL_WINDOWPOS_CENTERED = 0x2FFF0000;

// Flag for creating a visible window.
const SDL_WINDOW_SHOWN = 0x00000004;

if (!function_exists('Pnlx\\Func\\Libsdl\\SDL_Init')) {
    // @pnlx/autoload.php defines \Pnlx\Func functions only when features.use_functions is true.
    throw new RuntimeException('SDL global functions are disabled. Set pnl.json features.use_functions to true.');
}

// A tiny 5x7 bitmap font for the characters in "Hello World!".
// '1' marks a lit pixel; rows are top-to-bottom.
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

// Declare handles outside try so cleanup can see them.
$window = null;
$renderer = null;
$initialized = false;

try {
    // Start SDL's video subsystem through the global function.
    $result = SDL_Init(SDL_INIT_VIDEO);
    if ($result !== 0) {
        throw new RuntimeException('SDL_Init failed: ' . SDL_GetError());
    }
    $initialized = true;

    // Create a window and a renderer to draw into it.
    $window = SDL_CreateWindow(
        'Hello World!',
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        640,
        360,
        SDL_WINDOW_SHOWN
    );
    if (is_null($window)) {
        // is_null() hides the raw FFI::isNull() check.
        throw new RuntimeException('SDL_CreateWindow failed: ' . SDL_GetError());
    }

    $renderer = SDL_CreateRenderer($window, -1, 0);
    if (is_null($renderer)) {
        throw new RuntimeException('SDL_CreateRenderer failed: ' . SDL_GetError());
    }

    // Clear to a dark background.
    SDL_SetRenderDrawColor($renderer, 0x1E, 0x1E, 0x1E, 0xFF);
    SDL_RenderClear($renderer);

    // Draw "Hello World!" in the window, scaling each font pixel into a block.
    // SDL_RenderDrawPoint takes only integers, so no FFI structs are needed.
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
        $x += 6 * $scale; // 5px glyph + 1px gap
    }

    // Present the frame and keep the window up briefly while pumping events.
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
    // Destroy the window if creation succeeded.
    if (!is_null($window)) {
        SDL_DestroyWindow($window);
    }

    // Quit SDL only if initialization succeeded.
    if ($initialized) {
        SDL_Quit();
    }
}
```


## Generated Files

Each generated PHP/Rust file starts with a header comment that records:

- the generation timestamp,
- the host it was generated on,
- the generator's OS/architecture,
- the PHP version.

Generated package files may be overwritten whenever they're regenerated. If you want to change behavior by hand, add overrides under `src/` instead of editing `src/generated` directly.
