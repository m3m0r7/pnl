<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\Runtime;
use Pnlx\Tests\Support\RuntimeWorkspace;
use Pnlx\Verifier;

class RuntimeTest extends TestCase
{
    private const EXAMPLE_CLASS = 'Pnlx\\Example\\Example';

    private static RuntimeWorkspace $workspace;

    public static function setUpBeforeClass(): void
    {
        self::$workspace = RuntimeWorkspace::create();
    }

    public static function tearDownAfterClass(): void
    {
        if (isset(self::$workspace)) {
            self::$workspace->remove();
        }
    }

    protected function setUp(): void
    {
        if (!class_exists(\FFI::class) || ini_get('ffi.enable') === '0') {
            self::markTestSkipped('PHP FFI is not enabled.');
        }
    }

    public function testVerifierAcceptsCurrentFFIConfigurationAndSchemas(): void
    {
        Verifier::shouldEnabledFFI();
        Verifier::shouldMatchSchema('pnl', self::$workspace->projectRoot . '/pnl.json');
        Verifier::shouldMatchSchema('pnlx', self::$workspace->installedPackageRoot . '/pnlx.json');
        Verifier::shouldMatchSchema('pnlx-pathmap', self::$workspace->projectRoot . '/@pnlx/pnlx-pathmap.json');

        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/pnlx-lock.json');
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/autoload.php');
        self::assertFileExists(self::$workspace->installedPackageRoot . '/README.md');
        self::assertFileDoesNotExist(self::$workspace->installedPackageRoot . '/src/generated/stale.php');
    }

    public function testInstallGeneratesExpectedWorkspaceArtifacts(): void
    {
        $generated = self::$workspace->installedPackageRoot . '/src/generated';
        foreach ([
            'index.php',
            'functions.php',
            'Example.php',
            'ExampleContext.php',
            'function.aliases.php',
            'example.ffi.php',
            'example.bridge.rs',
        ] as $file) {
            self::assertFileExists($generated . '/' . $file);
        }

        // Global helpers are generated under the \Pnlx\Func\<Class> namespace.
        $functions = (string) file_get_contents($generated . '/functions.php');
        self::assertStringContainsString('namespace Pnlx\\Func\\Example;', $functions);
        self::assertStringContainsString("function_exists('Pnlx\\\\Func\\\\Example\\\\example_add')", $functions);

        // The workspace itself contains the lock, pathmap, and per-package bridge.
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/pnlx-pathmap.json');
        self::assertFileExists(self::$workspace->installedPackageRoot . '/bridge');
    }

    public function testInstallGeneratesPnlJsonAndLockfile(): void
    {
        // pnl.json records the installed extension.
        $pnl = self::$workspace->pnlManifest();
        self::assertSame('2026-07-01', $pnl['schema_version']);
        $extensions = $pnl['extensions'];
        self::assertIsArray($extensions);
        self::assertArrayHasKey('example/example', $extensions);
        $entry = $extensions['example/example'];
        self::assertIsArray($entry);
        self::assertTrue($entry['required']);

        // pnlx-lock.json pins the resolved version, source, integrity signature,
        // and native libraries for this platform.
        $extension = self::$workspace->lockExtension('example/example');
        self::assertSame('1.2.3', $extension['version']);
        self::assertSame('=1.2.3', $extension['constraint']);

        $source = $extension['source'];
        self::assertIsArray($source);
        self::assertSame('file', $source['type']);

        $dist = $extension['dist'];
        self::assertIsArray($dist);
        $distSha = $dist['sha256'];
        self::assertIsString($distSha);
        self::assertMatchesRegularExpression('/^[0-9a-f]{64}$/', $distSha);

        $requires = $extension['requires'];
        self::assertIsArray($requires);
        self::assertArrayHasKey('example', $requires);
        $native = $requires['example'];
        self::assertIsArray($native);
        $nativeVersion = $native['version'];
        self::assertIsString($nativeVersion);
        self::assertMatchesRegularExpression('/^\d+\.\d+\.\d+$/', $nativeVersion);
        $nativeSha = $native['sha256'];
        self::assertIsString($nativeSha);
        self::assertMatchesRegularExpression('/^[0-9a-f]{64}$/', $nativeSha);
    }

    /**
     * Golden test: the generated @pnlx source files must match the committed
     * snapshots exactly (after normalizing the volatile metadata header), so any
     * unintended change to the generated content is caught.
     *
     * Regenerate the snapshots with: UPDATE_GOLDEN=1 vendor/bin/phpunit
     */
    public function testGeneratedArtifactsMatchGoldenSnapshots(): void
    {
        $generatedDir = self::$workspace->installedPackageRoot . '/src/generated';
        $goldenDir = __DIR__ . '/golden/example';
        $files = [
            'index.php',
            'functions.php',
            'Example.php',
            'ExampleContext.php',
            'function.aliases.php',
            'example.ffi.php',
            'example.bridge.rs',
        ];

        $updating = getenv('UPDATE_GOLDEN') === '1';
        if ($updating && !is_dir($goldenDir)) {
            mkdir($goldenDir, 0o777, true);
        }

        foreach ($files as $file) {
            $generatedPath = $generatedDir . '/' . $file;
            self::assertFileExists($generatedPath);
            $actual = self::normalizeGenerated((string) file_get_contents($generatedPath));

            $goldenPath = $goldenDir . '/' . $file;
            if ($updating) {
                file_put_contents($goldenPath, $actual);
                continue;
            }

            self::assertFileExists($goldenPath, "Missing golden for {$file}; run UPDATE_GOLDEN=1 phpunit.");
            self::assertSame(
                self::normalizeGenerated((string) file_get_contents($goldenPath)),
                $actual,
                "Generated {$file} no longer matches its golden snapshot."
            );
        }

        if ($updating) {
            self::markTestSkipped('Updated golden snapshots.');
        }
    }

    private static function normalizeGenerated(string $content): string
    {
        // Blank out the per-run metadata header values (timestamp/host/OS/PHP).
        $normalized = preg_replace(
            '/((?:Generated at|Generated on|Generator OS|PHP version): ).*/',
            '$1<normalized>',
            $content
        );

        return $normalized ?? $content;
    }

    public function testRuntimeLoadsExampleThroughCompiledBridge(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $example = $runtime->load(self::EXAMPLE_CLASS);

        self::assertSame('1.2.3', $example->{'example_version'}());
        self::assertSame('1.2.3', $example->example_version());
        self::assertSame(5, $example->{'example_add'}(2, 3));
        self::assertSame(5, $example->example_add(2, 3));
        self::assertSame(5, $example->{'exampleAdd'}(2, 3));
        self::assertSame(5, $example->{'ExampleAdd'}(2, 3));
    }

    public function testRuntimeCanExposeGeneratedGlobalFunctions(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->load(self::EXAMPLE_CLASS);

        self::assertTrue(Runtime::enableFunctions(self::$workspace->projectRoot));
        // Generated global functions live under the \Pnlx\Func\<Class> namespace.
        self::assertFalse(function_exists('example_add'));
        self::assertTrue(function_exists('Pnlx\\Func\\Example\\example_add'));
        self::assertSame(5, \Pnlx\Func\Example\example_add(2, 3));
        self::assertStringContainsString(
            "require_once __DIR__ . '/packages/example/example/1.2.3/src/generated/index.php';",
            (string) file_get_contents(self::$workspace->projectRoot . '/@pnlx/autoload.php')
        );
        self::assertStringNotContainsString(
            'global $runtime_',
            (string) file_get_contents(self::$workspace->installedPackageRoot . '/src/generated/index.php')
        );
    }

    public function testGeneratedEntrypointRunsPreloadAndPostloadHooks(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->load(self::EXAMPLE_CLASS);

        self::assertTrue($GLOBALS['pnlx_test_preload_runtime_available'] ?? false);
        self::assertTrue($GLOBALS['pnlx_test_postload_runtime_available'] ?? false);
        self::assertIsString($GLOBALS['pnlx_test_preload_runtime_var_name'] ?? null);
        self::assertSame(
            $GLOBALS['pnlx_test_preload_runtime_var_name'],
            $GLOBALS['pnlx_test_postload_runtime_var_name'] ?? null
        );
        self::assertTrue($GLOBALS['pnlx_test_postload_entity_loaded'] ?? false);
    }

    public function testRuntimeReturnsBridgeContextByClass(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $context = $runtime->context(self::EXAMPLE_CLASS);

        self::assertSame('example/example', $context->name());
        self::assertSame('1.2.3', $context->version());
        self::assertSame(hash_file('sha256', self::$workspace->bridgeLibraryPath), $context->hash());
        self::assertSame(self::$workspace->bridgeLibraryPath, $context->path());
    }

    public function testGeneratedEntityShapeIsOverrideFriendly(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $example = $runtime->load(self::EXAMPLE_CLASS);
        $reflection = new \ReflectionObject($example);

        self::assertFalse($reflection->isFinal());
        self::assertFalse($reflection->hasMethod('__get'));
        self::assertFalse($reflection->hasMethod('cdefPath'));
        self::assertFalse($reflection->hasMethod('aliasesPath'));
        self::assertTrue($reflection->hasMethod('example_version'));
        self::assertTrue($reflection->hasMethod('exampleAdd'));
    }

    public function testRuntimeProvidesAllocatorAndStaticUtil(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $buffer = $runtime->allocator()->voidPointerArray(1);

        self::assertInstanceOf(\FFI\CData::class, $buffer);
        self::assertSame('ok', \Pnlx\Util::cString('ok'));
        // \Pnlx\Util\is_null() falls back to PHP's is_null for non-CData values.
        self::assertTrue(\Pnlx\Util\is_null(null));
        self::assertFalse(\Pnlx\Util\is_null('not cdata'));
    }

    public function testPnlxBuildRebuildsInstalledBridge(): void
    {
        unlink(self::$workspace->bridgeLibraryPath);

        self::$workspace->rebuildBridges(['example', 'example/example']);

        self::assertFileExists(self::$workspace->bridgeLibraryPath);
        self::assertSame(
            hash_file('sha256', self::$workspace->bridgeLibraryPath),
            self::$workspace->pathmapBridge('example')['sha256']
        );
    }

    public function testGeneratedRustBridgeCanCallExampleLibrary(): void
    {
        self::$workspace->runBridgeCheck(
            self::$workspace->installedPackageRoot . '/src/generated/example.bridge.rs'
        );
    }

    public function testInstallRecordsCompiledBridgeInPathmap(): void
    {
        $bridge = self::$workspace->pathmapBridge('example');

        self::assertSame(
            '@pnlx/packages/example/example/1.2.3/bridge/example.bridge.rs',
            $bridge['source']
        );
        self::assertSame(
            hash_file('sha256', self::$workspace->bridgeLibraryPath),
            $bridge['sha256']
        );
    }
}
