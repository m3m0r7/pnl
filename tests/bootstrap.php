<?php

declare(strict_types=1);

/*
 * PHPUnit bootstrap.
 *
 * Composer provides only the dev tooling and the `Pnlx\Tests\` test classes; the
 * SDK itself (`Pnlx\`) is loaded through its own Composer-free autoloader, the
 * same mechanism used at runtime. This keeps the SDK from depending on Composer
 * even under test.
 */

require __DIR__ . '/../vendor/autoload.php';
require __DIR__ . '/../src/sdk/autoload.php';
