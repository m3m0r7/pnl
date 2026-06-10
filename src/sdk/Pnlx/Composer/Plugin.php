<?php

declare(strict_types=1);

namespace Pnlx\Composer;

use Composer\Composer;
use Composer\EventDispatcher\EventSubscriberInterface;
use Composer\IO\IOInterface;
use Composer\Plugin\PluginInterface;
use Composer\Script\ScriptEvents;

/**
 * Composer plugin entry point for `composer require m3m0r7/pnl`.
 *
 * After every install/update it hands off to {@see BinaryInstaller}, which
 * builds the bundled Rust CLI sources (`make validate` as the pre-build gate,
 * then `make build` + `make install`) and links `pnl` / `pnlx` into the
 * project's composer bin directory.
 *
 * The post-command events are used (rather than per-package pre/post events)
 * because on the very first `composer require` the package files are not on
 * disk — and the plugin itself is not activated — until after its own package
 * operation completes.
 */
final class Plugin implements PluginInterface, EventSubscriberInterface
{
    private ?Composer $composer = null;
    private ?IOInterface $io = null;

    public function activate(Composer $composer, IOInterface $io): void
    {
        $this->composer = $composer;
        $this->io = $io;
    }

    public function deactivate(Composer $composer, IOInterface $io): void
    {
    }

    public function uninstall(Composer $composer, IOInterface $io): void
    {
    }

    /**
     * @return array<string, string>
     */
    public static function getSubscribedEvents(): array
    {
        return [
            ScriptEvents::POST_INSTALL_CMD => 'installBinaries',
            ScriptEvents::POST_UPDATE_CMD => 'installBinaries',
        ];
    }

    public function installBinaries(): void
    {
        if ($this->composer === null || $this->io === null) {
            return;
        }

        (new BinaryInstaller($this->composer, $this->io))->run();
    }
}
