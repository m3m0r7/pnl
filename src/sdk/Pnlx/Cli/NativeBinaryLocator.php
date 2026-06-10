<?php

declare(strict_types=1);

namespace Pnlx\Cli;

use RuntimeException;

/**
 * Resolves the native `pnl` / `pnlx` binaries on demand for the `bin/` shims.
 *
 * pnl ships as an ordinary composer library (no plugin, so no `allow-plugins`
 * prompt). The native binaries are produced lazily the first time the CLI runs:
 * built from the bundled Rust sources when a toolchain is present, and otherwise
 * downloaded from the GitHub release that matches the bundled version. The
 * result is cached under `bin/.native/<version>/` so later runs exec it directly.
 *
 * This class is intentionally dependency-free so the `bin/` shims can `require`
 * it without an autoloader being available yet.
 */
final class NativeBinaryLocator
{
    public const REPOSITORY = 'https://github.com/m3m0r7/pnl';

    /** Binaries shipped by this package. */
    private const BINARIES = ['pnl', 'pnlx'];

    /**
     * Ensure the native `$name` binary exists for the bundled source version and
     * return its absolute path, building or downloading it on first use.
     */
    public static function ensure(string $packageRoot, string $name): string
    {
        $version = self::version($packageRoot);
        $dir = $packageRoot . '/bin/.native/' . $version;
        $binary = $dir . '/' . self::executableName($name);
        if (is_executable($binary)) {
            return $binary;
        }

        if (!is_dir($dir) && !@mkdir($dir, 0o755, true) && !is_dir($dir)) {
            throw new RuntimeException(sprintf('pnl: failed to create %s', $dir));
        }

        // Prefer a from-source build (guaranteed ABI match); fall back to the
        // prebuilt release binary when no Rust toolchain is available.
        if (!self::tryBuild($packageRoot, $dir)) {
            self::download($version, $dir);
        }

        // The binary was just produced; drop the stat cache so the freshly
        // created file is seen.
        clearstatcache(true, $binary);
        if (!is_executable($binary)) {
            throw new RuntimeException(sprintf(
                'pnl: the %s binary is still missing after build/download; '
                . 'install a Rust toolchain (https://rustup.rs) or download a release from %s/releases',
                $name,
                self::REPOSITORY,
            ));
        }
        return $binary;
    }

    /** The bundled package version, read from Cargo.toml. */
    public static function version(string $packageRoot): string
    {
        $cargoToml = @file_get_contents($packageRoot . '/Cargo.toml');
        $version = $cargoToml === false ? null : self::cargoVersion($cargoToml);
        if ($version === null) {
            throw new RuntimeException('pnl: could not read the package version from Cargo.toml');
        }
        return $version;
    }

    /** The `version = "…"` value from a Cargo.toml document. */
    public static function cargoVersion(string $cargoToml): ?string
    {
        return preg_match('/^version\s*=\s*"([^"]+)"/m', $cargoToml, $matches) === 1
            ? $matches[1]
            : null;
    }

    /** The Rust target triple matching the current PHP runtime. */
    public static function rustTarget(): string
    {
        $machine = strtolower(php_uname('m'));
        $arch = (str_contains($machine, 'arm64') || str_contains($machine, 'aarch64'))
            ? 'aarch64'
            : 'x86_64';

        return match (PHP_OS_FAMILY) {
            'Darwin' => $arch . '-apple-darwin',
            'Windows' => 'x86_64-pc-windows-msvc',
            default => 'x86_64-unknown-linux-gnu',
        };
    }

    private static function executableName(string $name): string
    {
        return PHP_OS_FAMILY === 'Windows' ? $name . '.exe' : $name;
    }

    /**
     * Build both binaries from the bundled Rust sources, copying them into
     * `$dir`. Returns false (so the caller falls back to a download) when the
     * toolchain is missing or the build fails.
     */
    private static function tryBuild(string $packageRoot, string $dir): bool
    {
        if (!is_file($packageRoot . '/Cargo.toml')) {
            return false;
        }
        if (self::run(['cargo', '--version'], $packageRoot, true) !== 0) {
            return false;
        }

        fwrite(STDERR, "pnl: building native binaries from source (first run; this may take a few minutes)\n");
        if (self::run(['cargo', 'build', '--release', '--bins', '--locked'], $packageRoot) !== 0) {
            fwrite(STDERR, "pnl: source build failed; falling back to a prebuilt release\n");
            return false;
        }

        $release = $packageRoot . '/target/release';
        foreach (self::BINARIES as $binary) {
            $name = self::executableName($binary);
            if (!@copy($release . '/' . $name, $dir . '/' . $name)) {
                return false;
            }
            @chmod($dir . '/' . $name, 0o755);
        }
        return true;
    }

    /** Download the matching release archive and unpack the binaries into `$dir`. */
    private static function download(string $version, string $dir): void
    {
        $target = self::rustTarget();
        $isWindows = PHP_OS_FAMILY === 'Windows';
        $package = sprintf('pnl-%s-%s', $version, $target);
        $archiveName = $package . ($isWindows ? '.zip' : '.tar.gz');
        $url = sprintf('%s/releases/download/v%s/%s', self::REPOSITORY, $version, $archiveName);

        fwrite(STDERR, sprintf("pnl: downloading prebuilt binaries from %s\n", $url));
        $data = @file_get_contents($url);
        if ($data === false) {
            throw new RuntimeException(sprintf(
                'pnl: failed to download %s. Install a Rust toolchain (https://rustup.rs) to build from '
                . 'source, or download a release manually from %s/releases',
                $url,
                self::REPOSITORY,
            ));
        }

        $work = self::tempDir();
        $archivePath = $work . '/' . $archiveName;
        file_put_contents($archivePath, $data);
        self::unpack($archivePath, $work, $isWindows);

        // Release archives wrap the binaries in a `pnl-<version>-<target>/` dir.
        foreach (self::BINARIES as $binary) {
            $name = self::executableName($binary);
            $from = $work . '/' . $package . '/' . $name;
            if (!is_file($from)) {
                throw new RuntimeException(sprintf('pnl: %s is missing from the downloaded archive', $name));
            }
            if (!@copy($from, $dir . '/' . $name)) {
                throw new RuntimeException(sprintf('pnl: failed to install %s into %s', $name, $dir));
            }
            @chmod($dir . '/' . $name, 0o755);
        }

        self::removeTree($work);
    }

    private static function unpack(string $archivePath, string $destination, bool $isWindows): void
    {
        if ($isWindows) {
            $zip = new \ZipArchive();
            if ($zip->open($archivePath) !== true || !$zip->extractTo($destination)) {
                throw new RuntimeException(sprintf('pnl: failed to extract %s', $archivePath));
            }
            $zip->close();
            return;
        }

        // `tar` is available on macOS and Linux and handles .tar.gz directly.
        if (self::run(['tar', '-xzf', $archivePath, '-C', $destination], $destination) !== 0) {
            throw new RuntimeException(sprintf('pnl: failed to extract %s', $archivePath));
        }
    }

    /**
     * Run a command, returning its exit code. Child stdout/stderr go to this
     * process's stderr (keeping the CLI's stdout clean); `$quiet` discards both
     * for existence probes.
     *
     * @param list<string> $command
     */
    private static function run(array $command, string $cwd, bool $quiet = false): int
    {
        $null = PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null';
        $descriptors = $quiet
            ? [['file', $null, 'r'], ['file', $null, 'w'], ['file', $null, 'w']]
            : [STDIN, STDERR, STDERR];

        $process = @proc_open($command, $descriptors, $pipes, $cwd);
        if (!is_resource($process)) {
            return 127;
        }
        foreach ($pipes as $pipe) {
            if (is_resource($pipe)) {
                fclose($pipe);
            }
        }
        return proc_close($process);
    }

    private static function tempDir(): string
    {
        $dir = sys_get_temp_dir() . '/pnl-native-' . bin2hex(random_bytes(6));
        if (!@mkdir($dir, 0o755, true) && !is_dir($dir)) {
            throw new RuntimeException(sprintf('pnl: failed to create %s', $dir));
        }
        return $dir;
    }

    private static function removeTree(string $dir): void
    {
        if (!is_dir($dir)) {
            return;
        }
        $items = new \RecursiveIteratorIterator(
            new \RecursiveDirectoryIterator($dir, \FilesystemIterator::SKIP_DOTS),
            \RecursiveIteratorIterator::CHILD_FIRST,
        );
        foreach ($items as $item) {
            if (!$item instanceof \SplFileInfo) {
                continue;
            }
            $item->isDir() ? @rmdir($item->getPathname()) : @unlink($item->getPathname());
        }
        @rmdir($dir);
    }
}
