<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Runtime;

class RuntimeComposeTest extends TestCase
{
    public function testComposeRequiresAtLeastTwoExtensions(): void
    {
        $this->expectException(ExtensionLoadException::class);
        $this->expectExceptionMessage('at least two');

        Runtime::compose([]);
    }
}
