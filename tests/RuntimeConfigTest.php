<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\RuntimeConfig;
use Pnlx\Tests\Support\Filesystem;

class RuntimeConfigTest extends TestCase
{
    private string $projectRoot;

    protected function setUp(): void
    {
        $this->projectRoot = sys_get_temp_dir() . '/pnlx-config-' . bin2hex(random_bytes(6));
        if (!mkdir($this->projectRoot) && !is_dir($this->projectRoot)) {
            self::fail('Failed to create temporary project root.');
        }
    }

    protected function tearDown(): void
    {
        if (is_dir($this->projectRoot)) {
            Filesystem::removeDirectory($this->projectRoot);
        }
    }

    public function testOutputDirDefaultsToAtPnlxWithoutManifest(): void
    {
        $config = new RuntimeConfig($this->projectRoot);

        self::assertSame('@pnlx', $config->outputDir());
        self::assertSame('@pnlx/pnlx-pathmap.json', $config->pathmapFile());
    }

    public function testOutputDirHonorsManifestSetting(): void
    {
        $this->writeManifest('build/workspace');
        $config = new RuntimeConfig($this->projectRoot);

        self::assertSame('build/workspace', $config->outputDir());
        self::assertSame('build/workspace/pnlx-pathmap.json', $config->pathmapFile());
    }

    public function testOutputDirFallsBackWhenManifestSettingIsEmpty(): void
    {
        $this->writeManifest('');
        $config = new RuntimeConfig($this->projectRoot);

        self::assertSame('@pnlx', $config->outputDir());
    }

    private function writeManifest(string $outputDir): void
    {
        $manifest = [
            'schema_version' => '2026-07-01',
            'repositories' => [],
            'library_paths' => [],
            'output_dir' => $outputDir,
            'extensions' => new \stdClass(),
        ];
        file_put_contents(
            $this->projectRoot . '/pnl.json',
            json_encode($manifest, JSON_THROW_ON_ERROR | JSON_PRETTY_PRINT)
        );
    }
}
