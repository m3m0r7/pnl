<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use JsonException;
use PHPUnit\Framework\TestCase;

final class ComposerPackageTest extends TestCase
{
    /**
     * @throws JsonException
     */
    public function testDeclaredBinariesExistAndAreExecutable(): void
    {
        $packageRoot = dirname(__DIR__);
        $composerJson = file_get_contents($packageRoot . '/composer.json');

        self::assertNotFalse($composerJson);

        /** @var array{bin: list<string>} $manifest */
        $manifest = json_decode($composerJson, true, flags: JSON_THROW_ON_ERROR);

        self::assertSame(['bin/pnl', 'bin/pnlx'], $manifest['bin']);

        foreach ($manifest['bin'] as $binary) {
            $path = $packageRoot . '/' . $binary;
            self::assertFileExists($path);
            self::assertFileIsReadable($path);

            if (PHP_OS_FAMILY !== 'Windows') {
                self::assertTrue(is_executable($path), sprintf('%s must be executable', $binary));
            }
        }
    }
}
