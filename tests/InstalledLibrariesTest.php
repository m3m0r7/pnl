<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\Installed\InstalledLibraries;

final class InstalledLibrariesTest extends TestCase
{
    private string $pathmap;

    protected function setUp(): void
    {
        $this->pathmap = tempnam(sys_get_temp_dir(), 'pnlx-pathmap-') ?: '';
        self::assertNotSame('', $this->pathmap);
        file_put_contents($this->pathmap, json_encode([
            'headers' => [
                'sdl2' => ['path' => '/opt/include/SDL2/SDL.h', 'sha256' => str_repeat('a', 64)],
            ],
            'native_libraries' => [
                'sdl2' => [
                    'resolved_name' => 'libSDL2.dylib',
                    'path' => '/opt/lib/libSDL2.dylib',
                    'version' => '2.32.10',
                    'sha256' => str_repeat('b', 64),
                    'installed_at' => '2026-06-10T04:18:33Z',
                ],
                'usb' => [
                    'resolved_name' => 'libusb-1.0.dylib',
                    'path' => '/opt/lib/libusb-1.0.dylib',
                    'version' => '1.0.29',
                    'sha256' => str_repeat('c', 64),
                ],
            ],
        ], JSON_THROW_ON_ERROR));
    }

    protected function tearDown(): void
    {
        if (is_file($this->pathmap)) {
            unlink($this->pathmap);
        }
    }

    public function testNativeLibrariesProjectsThePathmap(): void
    {
        $libraries = InstalledLibraries::nativeLibraries($this->pathmap);

        self::assertCount(2, $libraries);

        $sdl = $libraries[0];
        self::assertSame('2.32.10', $sdl['version']);
        self::assertSame('sdl2', $sdl['name']);
        self::assertSame(str_repeat('b', 64), $sdl['hash']);
        self::assertSame('/opt/include/SDL2/SDL.h', $sdl['paths']['header']);
        self::assertSame('/opt/lib/libSDL2.dylib', $sdl['paths']['library']);
        self::assertSame('2026-06-10T04:18:33Z', $sdl['installed_at']);

        // A native library with no header / no install timestamp degrades to null.
        $usb = $libraries[1];
        self::assertNull($usb['paths']['header']);
        self::assertNull($usb['installed_at']);
    }

    public function testNativeLibrariesIsEmptyForAMissingPathmap(): void
    {
        self::assertSame([], InstalledLibraries::nativeLibraries('/no/such/pathmap.json'));
    }

    /**
     * @return array<string, array{version: string, classes: list<string>}>
     */
    private static function packages(): array
    {
        return [
            'libsdl/libsdl' => ['version' => '2.32.10', 'classes' => ['Pnlx\\Libsdl\\Libsdl']],
            'libusb/libusb' => ['version' => '1.0.29', 'classes' => ['Pnlx\\Libusb\\Libusb']],
        ];
    }

    public function testIsInstalledByFullName(): void
    {
        self::assertTrue(InstalledLibraries::isInstalled(self::packages(), 'libsdl/libsdl'));
        self::assertFalse(InstalledLibraries::isInstalled(self::packages(), 'acme/missing'));
    }

    public function testIsInstalledByBareLeaf(): void
    {
        self::assertTrue(InstalledLibraries::isInstalled(self::packages(), 'libusb'));
    }

    public function testIsInstalledByClassName(): void
    {
        self::assertTrue(InstalledLibraries::isInstalled(self::packages(), 'Pnlx\\Libsdl\\Libsdl'));
        // A leading backslash (as `::class` never emits, but defensively) is fine.
        self::assertTrue(InstalledLibraries::isInstalled(self::packages(), '\\Pnlx\\Libsdl\\Libsdl'));
        self::assertFalse(InstalledLibraries::isInstalled(self::packages(), 'Pnlx\\Other\\Thing'));
    }

    public function testIsInstalledHonoursVersionConstraint(): void
    {
        $packages = self::packages();
        self::assertTrue(InstalledLibraries::isInstalled($packages, 'libsdl/libsdl', '>2.0.0 & <3.0.0'));
        self::assertFalse(InstalledLibraries::isInstalled($packages, 'libsdl/libsdl', '>=3.0.0'));
        self::assertTrue(InstalledLibraries::isInstalled($packages, 'libusb/libusb', '^1.0.0'));
    }

    public function testIsInstalledFromLockReadsExtensionsAndClasses(): void
    {
        $lock = tempnam(sys_get_temp_dir(), 'pnlx-lock-') ?: '';
        self::assertNotSame('', $lock);

        try {
            file_put_contents($lock, json_encode([
                'extensions' => [
                    'libsdl/libsdl' => [
                        'version' => '2.32.10',
                        'classes' => ['Pnlx\\Libsdl\\Libsdl'],
                    ],
                    'libusb/libusb' => [
                        'version' => '1.0.29',
                        'classes' => ['Pnlx\\Libusb\\Libusb'],
                    ],
                ],
            ], JSON_THROW_ON_ERROR));

            // By full name, bare leaf, and generated class name.
            self::assertTrue(InstalledLibraries::isInstalledFromLock($lock, 'libsdl/libsdl'));
            self::assertTrue(InstalledLibraries::isInstalledFromLock($lock, 'libusb'));
            self::assertTrue(InstalledLibraries::isInstalledFromLock($lock, 'Pnlx\\Libsdl\\Libsdl'));
            self::assertFalse(InstalledLibraries::isInstalledFromLock($lock, 'acme/missing'));

            // Version constraints are honoured from the lock's recorded version.
            self::assertTrue(InstalledLibraries::isInstalledFromLock($lock, 'libsdl/libsdl', '>2.0.0 & <3.0.0'));
            self::assertFalse(InstalledLibraries::isInstalledFromLock($lock, 'libsdl/libsdl', '>=3.0.0'));
        } finally {
            if (is_file($lock)) {
                unlink($lock);
            }
        }
    }

    public function testIsInstalledFromLockIsFalseForMissingLock(): void
    {
        self::assertFalse(InstalledLibraries::isInstalledFromLock('/no/such/lock.json', 'libsdl/libsdl'));
    }
}
