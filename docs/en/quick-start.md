# Quick Start

[← Documentation index](../../README.md) · [日本語](../ja/quick-start.md)

## Quick Start

The smallest possible example: call C's `printf` from PHP. The C standard library (`libc`) ships with every operating system — macOS, Linux, and Windows — so there is **nothing to install** beyond pnl itself.

In your project directory:

```sh
# 1. Create pnl.json. The official package repository is built in, so a bare
#    name resolves even with no "repositories" configured.
pnl init

# 2. Add the libc package (resolved from the built-in default repository).
pnl install libc
```

Then call it from PHP (`quickstart.php`):

```php
<?php

declare(strict_types=1);

// One require: the SDK loads itself (no Composer autoloader needed at runtime).
require_once __DIR__ . '/@pnlx/autoload.php';

use Pnlx\Libc\Libc;

// A C library is a bag of functions, so entities are called statically —
// no instantiation. The first call boots the extension automatically.
Libc::printf("Hello, World from libc!\n");
Libc::puts("And this line is printed by libc puts.");
```

```sh
php quickstart.php
```

```text
Hello, World from libc!
And this line is printed by libc puts.
```

Why `libc` is a good first package: its functions (`printf`, `puts`) come from the C runtime that is already part of the OS, so no `brew install` / `apt-get install` step is needed. Its library entries are declared `"virtual": true` in the package, which tells pnl to link them by name without expecting a file on disk (on macOS, libc lives only in the dyld shared cache). When you are ready for a real library, try [`pnl install libusb`](commands.md#pnl-install-source).
