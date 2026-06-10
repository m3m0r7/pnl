<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\Cli\NativeBinaryLocator;

final class NativeBinaryLocatorTest extends TestCase
{
    public function testParsesTheCargoPackageVersion(): void
    {
        $cargoToml = <<<'TOML'
            [package]
            name = "pnl"
            version = "0.1.5"
            edition = "2024"
            TOML;

        self::assertSame('0.1.5', NativeBinaryLocator::cargoVersion($cargoToml));
        self::assertNull(NativeBinaryLocator::cargoVersion('[package]'));
    }

    public function testBundledCargoTomlVersionIsParseable(): void
    {
        $cargoToml = file_get_contents(__DIR__ . '/../Cargo.toml');

        self::assertNotFalse($cargoToml);
        self::assertNotNull(NativeBinaryLocator::cargoVersion($cargoToml));
    }

    public function testResolvesAKnownReleaseTargetForThisRuntime(): void
    {
        self::assertContains(NativeBinaryLocator::rustTarget(), [
            'aarch64-apple-darwin',
            'x86_64-apple-darwin',
            'x86_64-unknown-linux-gnu',
            'x86_64-pc-windows-msvc',
        ]);
    }
}
