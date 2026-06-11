<?php

declare(strict_types=1);

namespace Pnlx\Installed;

use Pnlx\Version\VersionConstraint;

/**
 * Backs the global `pnl_installed_native_libraries()` and `pnl_is_installed()`
 * helpers declared in the generated `@pnlx/autoload.php`.
 *
 * The native-library listing is read straight from `@pnlx/pnlx-pathmap.json`;
 * the install check is answered from a package map the generator bakes into the
 * autoload file (extension name + version + generated class names), so neither
 * helper needs to boot the full runtime.
 */
final class InstalledLibraries
{
    /**
     * Every native library recorded in the pathmap.
     *
     * @return list<array{
     *     version: string,
     *     name: string,
     *     hash: string,
     *     paths: array{header: ?string, library: string},
     *     installed_at: ?string
     * }>
     */
    public static function nativeLibraries(string $pathmapPath): array
    {
        $pathmap = self::readJson($pathmapPath);
        $requires = self::section($pathmap, 'requires');
        $headers = self::section($pathmap, 'headers');

        $libraries = [];
        foreach ($requires as $name => $native) {
            if (!is_array($native)) {
                continue;
            }
            $header = $headers[(string) $name] ?? null;

            $libraries[] = [
                'version' => self::stringField($native, 'version'),
                'name' => (string) $name,
                'hash' => self::stringField($native, 'sha256'),
                'paths' => [
                    'header' => is_array($header) ? self::nullableField($header, 'path') : null,
                    'library' => self::stringField($native, 'path'),
                ],
                'installed_at' => self::nullableField($native, 'installed_at'),
            ];
        }

        return $libraries;
    }

    /**
     * Whether the extension identified by `$target` is installed, optionally
     * constrained to `$version`.
     *
     * `$target` accepts either a generated class name (e.g. `Libsdl::class`) or a
     * `vendor/package` name (e.g. `libsdl/libsdl`). `$version`, when given, is a
     * constraint expression such as `>3.0.0 & <4.0.0`.
     *
     * @param array<string, array{version: string, classes: list<string>}> $packages
     */
    public static function isInstalled(array $packages, string $target, ?string $version = null): bool
    {
        $package = self::resolvePackage($packages, $target);
        if ($package === null) {
            return false;
        }
        if ($version === null || $version === '') {
            return true;
        }

        return VersionConstraint::satisfies($package['version'], $version);
    }

    /**
     * @param array<string, array{version: string, classes: list<string>}> $packages
     * @return array{version: string, classes: list<string>}|null
     */
    private static function resolvePackage(array $packages, string $target): ?array
    {
        $needle = ltrim($target, '\\');

        // A class name (e.g. `Pnlx\Libsdl\Libsdl`) resolves through the class map.
        if (str_contains($needle, '\\')) {
            foreach ($packages as $package) {
                if (in_array($needle, $package['classes'], true)) {
                    return $package;
                }
            }

            return null;
        }

        // A full `vendor/package` name matches directly.
        if (isset($packages[$needle])) {
            return $packages[$needle];
        }

        // A bare leaf (e.g. `libsdl`) matches `vendor/libsdl`.
        foreach ($packages as $name => $package) {
            $slash = strrpos($name, '/');
            $leaf = $slash === false ? $name : substr($name, $slash + 1);
            if ($leaf === $needle) {
                return $package;
            }
        }

        return null;
    }

    /**
     * @return array<array-key, mixed>
     */
    private static function readJson(string $path): array
    {
        if (!is_file($path)) {
            return [];
        }
        $contents = file_get_contents($path);
        if ($contents === false) {
            return [];
        }
        $decoded = json_decode($contents, true);

        return is_array($decoded) ? $decoded : [];
    }

    /**
     * @param array<array-key, mixed> $data
     * @return array<array-key, mixed>
     */
    private static function section(array $data, string $key): array
    {
        $value = $data[$key] ?? null;

        return is_array($value) ? $value : [];
    }

    /**
     * @param array<array-key, mixed> $data
     */
    private static function stringField(array $data, string $key): string
    {
        $value = $data[$key] ?? '';

        return is_string($value) ? $value : '';
    }

    /**
     * @param array<array-key, mixed> $data
     */
    private static function nullableField(array $data, string $key): ?string
    {
        $value = $data[$key] ?? null;

        return is_string($value) ? $value : null;
    }
}
