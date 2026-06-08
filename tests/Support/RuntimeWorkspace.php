<?php

declare(strict_types=1);

namespace Pnlx\Tests\Support;

class RuntimeWorkspace
{
    public readonly string $projectRoot;

    public readonly string $packageSourceRoot;

    public readonly string $installedPackageRoot;

    public readonly string $nativeLibraryPath;

    public string $bridgeLibraryPath;

    private string $repoRoot;

    private function __construct()
    {
        $this->repoRoot = dirname(__DIR__, 2);
        $this->projectRoot = sys_get_temp_dir() . '/pnl-runtime-test-' . bin2hex(random_bytes(6));
        $this->packageSourceRoot = $this->projectRoot . '/packages/example';
        $this->installedPackageRoot = $this->projectRoot . '/@pnlx/packages/example/example';
        $this->nativeLibraryPath = $this->projectRoot . '/native/' . self::libraryName();
    }

    public static function create(): self
    {
        $workspace = new self();
        $workspace->copyPackageFixture();
        $workspace->compileNativeFixture();
        $workspace->writeProjectManifest();
        $workspace->installPackage();
        $workspace->writeGeneratedHooks();
        $workspace->bridgeLibraryPath = $workspace->resolveBridgeLibraryPath();

        return $workspace;
    }

    public function remove(): void
    {
        if (is_dir($this->projectRoot)) {
            Filesystem::removeDirectory($this->projectRoot);
        }
    }

    public function runBridgeCheck(string $bridgeSource): void
    {
        $check = $this->projectRoot . '/bridge-check.rs';
        $binary = $this->projectRoot . '/bridge-check';

        file_put_contents($check, sprintf(
            <<<'RS'
include!(%s);

fn main() {
    unsafe {
        assert_eq!(bridge::pnlx_bridge_example_add(2, 3), 5);
    }
}
RS,
            json_encode($bridgeSource, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES)
        ));

        $this->runner($this->projectRoot)->run([
            'rustc',
            $check,
            '-L',
            'native=' . dirname($this->nativeLibraryPath),
            '-l',
            'dylib=example',
            '-o',
            $binary,
        ]);

        $this->runner($this->projectRoot)->run([$binary], $this->nativeLibraryEnvironment());
    }

    public function rebuildBridge(string $package): void
    {
        $this->rebuildBridges([$package]);
    }

    /**
     * @param list<string> $packages
     */
    public function rebuildBridges(array $packages): void
    {
        $this->runner($this->projectRoot)->run([
            'cargo',
            'run',
            '--manifest-path',
            $this->repoRoot . '/Cargo.toml',
            '--bin',
            'pnlx',
            '--',
            'build',
            ...$packages,
        ]);

        $this->bridgeLibraryPath = $this->resolveBridgeLibraryPath();
    }

    /**
     * @return array<string, mixed>
     */
    public function pathmap(): array
    {
        return $this->readJson($this->projectRoot . '/@pnlx/pnlx-pathmap.json');
    }

    private function copyPackageFixture(): void
    {
        Filesystem::copyDirectory(
            $this->repoRoot . '/tests/fixtures/packages/example',
            $this->packageSourceRoot
        );

        mkdir($this->packageSourceRoot . '/src/generated', recursive: true);
        file_put_contents($this->packageSourceRoot . '/README.md', "example package\n");
        file_put_contents($this->packageSourceRoot . '/src/generated/stale.php', "<?php\n");
    }

    private function compileNativeFixture(): void
    {
        mkdir(dirname($this->nativeLibraryPath), recursive: true);
        $this->runner($this->projectRoot)->run([
            'rustc',
            '--crate-type',
            'cdylib',
            $this->repoRoot . '/tests/fixtures/native/example.rs',
            '-o',
            $this->nativeLibraryPath,
        ]);
    }

    private function writeProjectManifest(): void
    {
        $this->writeJson($this->projectRoot . '/pnl.json', [
            'schema_version' => '2026-07-01',
            'repositories' => [
                ['type' => 'file', 'url' => 'file://packages'],
            ],
            'load_paths' => [
                dirname($this->nativeLibraryPath),
            ],
            'enables' => [
                'use_functions' => true,
            ],
            'extensions' => [
                'example/example' => ['version' => '=1.2.3', 'required' => true],
            ],
        ]);
    }

    private function installPackage(): void
    {
        $this->runner($this->projectRoot)->run([
            'cargo',
            'run',
            '--manifest-path',
            $this->repoRoot . '/Cargo.toml',
            '--bin',
            'pnl',
            '--',
            'install',
            'packages/example',
        ]);
    }

    private function writeGeneratedHooks(): void
    {
        $generatedRoot = $this->installedPackageRoot . '/src/generated';
        file_put_contents($generatedRoot . '/preload.php', <<<'PHP'
<?php

$GLOBALS['pnlx_test_preload_runtime_available'] = isset($runtime) && $runtime instanceof \Pnlx\Runtime;
$GLOBALS['pnlx_test_preload_runtime_var_name'] = $runtimeVarName ?? null;
PHP);
        file_put_contents($generatedRoot . '/postload.php', <<<'PHP'
<?php

$GLOBALS['pnlx_test_postload_runtime_available'] = isset($runtime) && $runtime instanceof \Pnlx\Runtime;
$GLOBALS['pnlx_test_postload_runtime_var_name'] = $runtimeVarName ?? null;
$GLOBALS['pnlx_test_postload_entity_loaded'] = isset($runtimeVarName, $GLOBALS[$runtimeVarName]) && is_object($GLOBALS[$runtimeVarName]);
PHP);
    }

    private function resolveBridgeLibraryPath(): string
    {
        $pathmap = $this->pathmap();
        $library = $pathmap['bridges']['example']['library'];
        if (!is_string($library) || $library === '') {
            throw new \RuntimeException('Example bridge library is missing from pathmap.');
        }

        $path = realpath($this->absolutePath($library));
        if ($path === false) {
            throw new \RuntimeException('Example bridge library does not exist.');
        }

        return $path;
    }

    private function absolutePath(string $path): string
    {
        if (str_starts_with($path, '/')) {
            return $path;
        }

        return $this->projectRoot . '/' . $path;
    }

    /**
     * @return array<string, string>
     */
    private function nativeLibraryEnvironment(): array
    {
        return [
            'DYLD_LIBRARY_PATH' => dirname($this->nativeLibraryPath),
            'LD_LIBRARY_PATH' => dirname($this->nativeLibraryPath),
            'PATH' => getenv('PATH') ?: '',
        ];
    }

    private function runner(string $cwd): CommandRunner
    {
        return new CommandRunner($cwd);
    }

    /**
     * @param array<string, mixed> $data
     */
    private function writeJson(string $path, array $data): void
    {
        if (!is_dir(dirname($path))) {
            mkdir(dirname($path), recursive: true);
        }

        file_put_contents($path, json_encode($data, JSON_PRETTY_PRINT | JSON_THROW_ON_ERROR));
    }

    /**
     * @return array<string, mixed>
     */
    private function readJson(string $path): array
    {
        $data = json_decode((string) file_get_contents($path), true, flags: JSON_THROW_ON_ERROR);
        if (!is_array($data)) {
            throw new \RuntimeException(sprintf('%s did not contain a JSON object.', $path));
        }

        return $data;
    }

    private static function libraryName(): string
    {
        return match (PHP_OS_FAMILY) {
            'Darwin' => 'libexample.dylib',
            'Windows' => 'example.dll',
            default => 'libexample.so',
        };
    }
}
