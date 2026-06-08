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
        self::assertTrue(function_exists('example_add'));
        self::assertSame(5, example_add(2, 3));
        self::assertStringContainsString(
            "require_once __DIR__ . '/packages/example/example/src/generated/index.php';",
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

    public function testRuntimeProvidesUtilAndAllocator(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $buffer = $runtime->allocator()->voidPointerArray(1);

        self::assertInstanceOf(\FFI\CData::class, $buffer);
        self::assertInstanceOf(\Pnlx\Util::class, $runtime->utilities());
        self::assertSame('ok', $runtime->utilities()->cString('ok'));
        self::assertTrue(\Pnlx\Util::isNull(null));
        self::assertFalse(\Pnlx\Util::isNull('not cdata'));
    }

    public function testPnlxBuildRebuildsInstalledBridge(): void
    {
        unlink(self::$workspace->bridgeLibraryPath);

        self::$workspace->rebuildBridges(['example', 'example/example']);

        self::assertFileExists(self::$workspace->bridgeLibraryPath);
        self::assertSame(
            hash_file('sha256', self::$workspace->bridgeLibraryPath),
            self::$workspace->pathmap()['bridges']['example']['sha256']
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
        $pathmap = self::$workspace->pathmap();

        self::assertSame(
            '@pnlx/bridges/example/example.bridge.rs',
            $pathmap['bridges']['example']['source']
        );
        self::assertSame(
            hash_file('sha256', self::$workspace->bridgeLibraryPath),
            $pathmap['bridges']['example']['sha256']
        );
    }
}
