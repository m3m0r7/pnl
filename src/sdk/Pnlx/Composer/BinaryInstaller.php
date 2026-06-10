<?php

declare(strict_types=1);

namespace Pnlx\Composer;

use Composer\Composer;
use Composer\IO\IOInterface;
use Composer\Util\ProcessExecutor;
use RuntimeException;

/**
 * Builds and links the pnl/pnlx binaries when this package is installed as a
 * dependency.
 *
 * The flow mirrors a manual source install: `make validate` first confirms the
 * environment can build at all (Rust toolchain present), then `make build`
 * compiles the release binaries, and `make install BIN_DIR=<vendor/bin>` puts
 * the versioned binaries under `$PNL_HOME` and links them from the project's
 * composer bin directory. When the linked binary already reports the bundled
 * Cargo.toml version, everything is skipped.
 */
final class BinaryInstaller
{
    public const PACKAGE_NAME = 'm3m0r7/pnl';

    public function __construct(
        private readonly Composer $composer,
        private readonly IOInterface $io,
    ) {
    }

    public function run(): void
    {
        $package = $this->composer
            ->getRepositoryManager()
            ->getLocalRepository()
            ->findPackage(self::PACKAGE_NAME, '*');
        if ($package === null) {
            // Root-package development checkout: developers run make directly.
            return;
        }

        $path = $this->composer->getInstallationManager()->getInstallPath($package);
        if ($path === null || !is_file($path . '/Makefile') || !is_file($path . '/Cargo.toml')) {
            return;
        }

        $binDir = $this->composer->getConfig()->get('bin-dir');
        if (!is_string($binDir) || $binDir === '') {
            return;
        }
        if ($this->isCurrent($path, $binDir)) {
            $this->io->write('  <info>pnl</info> binaries are up to date');

            return;
        }

        // Pre-install gate: fail before compiling when the toolchain is missing.
        $this->runMake($path, 'validate');
        $this->io->write('  <info>pnl</info> building release binaries (this may take a few minutes)');
        $this->runMake($path, 'build');
        $this->runMake($path, 'install BIN_DIR=' . escapeshellarg($binDir));
        $this->io->write(sprintf('  <info>pnl</info> linked pnl and pnlx into %s', $binDir));
    }

    /**
     * Whether the linked binary already matches the bundled source version.
     */
    private function isCurrent(string $path, string $binDir): bool
    {
        $binary = $binDir . '/pnl';
        if (!is_executable($binary) || !is_executable($binDir . '/pnlx')) {
            return false;
        }

        $cargoToml = file_get_contents($path . '/Cargo.toml');
        $expected = $cargoToml === false ? null : self::cargoVersion($cargoToml);
        if ($expected === null) {
            return false;
        }

        $output = '';
        $status = (new ProcessExecutor($this->io))
            ->execute(escapeshellarg($binary) . ' version', $output);

        return $status === 0 && is_string($output) && trim($output) === $expected;
    }

    private function runMake(string $path, string $target): void
    {
        $command = sprintf('make -C %s %s', escapeshellarg($path), $target);
        $this->io->write(sprintf('  <info>pnl</info> running %s', $command));

        $process = new ProcessExecutor($this->io);
        if ($this->io->isInteractive()) {
            $status = $process->executeTty($command);
        } else {
            $output = '';
            $status = $process->execute($command, $output);
            if ($status !== 0 && is_string($output)) {
                $this->io->writeError($output);
            }
        }

        if ($status !== 0) {
            throw new RuntimeException(sprintf(
                'pnl: `%s` failed. Install the required toolchain (Rust via https://rustup.rs, make, a C compiler), '
                . 'or run the command manually, then re-run composer install.',
                $command,
            ));
        }
    }

    /**
     * The `version = "…"` value from a Cargo.toml document.
     */
    public static function cargoVersion(string $cargoToml): ?string
    {
        return preg_match('/^version\s*=\s*"([^"]+)"/m', $cargoToml, $matches) === 1
            ? $matches[1]
            : null;
    }
}
