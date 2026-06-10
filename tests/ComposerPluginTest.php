<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use Composer\Script\ScriptEvents;
use PHPUnit\Framework\TestCase;
use Pnlx\Composer\BinaryInstaller;
use Pnlx\Composer\Plugin;

final class ComposerPluginTest extends TestCase
{
    public function testSubscribesToPostInstallAndPostUpdate(): void
    {
        $events = Plugin::getSubscribedEvents();

        self::assertSame('installBinaries', $events[ScriptEvents::POST_INSTALL_CMD] ?? null);
        self::assertSame('installBinaries', $events[ScriptEvents::POST_UPDATE_CMD] ?? null);
    }

    public function testParsesTheCargoPackageVersion(): void
    {
        $cargoToml = <<<'TOML'
            [package]
            name = "pnl"
            version = "0.1.5"
            edition = "2024"
            TOML;

        self::assertSame('0.1.5', BinaryInstaller::cargoVersion($cargoToml));
        self::assertNull(BinaryInstaller::cargoVersion('[package]'));
    }

    public function testBundledCargoTomlVersionIsParseable(): void
    {
        $cargoToml = file_get_contents(__DIR__ . '/../Cargo.toml');

        self::assertNotFalse($cargoToml);
        self::assertNotNull(BinaryInstaller::cargoVersion($cargoToml));
    }
}
