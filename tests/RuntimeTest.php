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

    /**
     * The generated example entity as a class-string (it only exists once the
     * workspace has been installed, so phpstan cannot see it as a literal).
     *
     * @return class-string
     */
    private static function exampleClass(): string
    {
        // ltrim widens the literal-string constant to a general string, so the
        // class_exists() check below narrows it to a verified class-string.
        $class = ltrim(self::EXAMPLE_CLASS, '\\');
        if (!class_exists($class)) {
            self::fail("The example entity {$class} has not been generated yet.");
        }

        return $class;
    }

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

    public function testVerifierAcceptsCurrentFFIConfiguration(): void
    {
        // Schema validation is owned by the Rust toolchain (read_json validates on
        // install/validate); the PHP runtime only checks the FFI environment.
        Verifier::shouldEnabledFFI();

        self::assertFileExists(self::$workspace->projectRoot . '/pnlx-lock.json');
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/autoload.php');
        self::assertFileExists(self::$workspace->installedPackageRoot . '/README.md');
        self::assertFileDoesNotExist(self::$workspace->installedPackageRoot . '/src/generated/stale.php');

        // autoload.php exposes the built-in config.toml values and the build
        // target as PHP constants (PNL_CONFIG_*/PNLX_CONFIG_*, PNLX_BUILD_*).
        $autoload = (string) file_get_contents(self::$workspace->projectRoot . '/@pnlx/autoload.php');
        self::assertStringContainsString(
            "const PNLX_CONFIG_PACKAGES_REPOSITORY = 'https://github.com/m3m0r7/pnl-packages",
            $autoload
        );
        self::assertStringContainsString('const PNL_CONFIG_SCHEMA_VERSION =', $autoload);
        self::assertStringContainsString('const PNLX_BUILD_OS =', $autoload);
        self::assertStringContainsString('const PNLX_BUILD_ARCH =', $autoload);
        // The absolute pnl.json path captured at install time is baked in, so the
        // workspace loads regardless of the current directory.
        self::assertMatchesRegularExpression(
            "#const PNLX_PROJECT_MANIFEST = '[^']*/pnl\\.json';#",
            $autoload
        );

        // The pathmap records the absolute pnl.json path it was generated from.
        $pathmap = self::$workspace->pathmap();
        self::assertArrayHasKey('manifest', $pathmap);
        $manifest = $pathmap['manifest'];
        self::assertIsString($manifest);
        self::assertStringEndsWith('/pnl.json', $manifest);
    }

    public function testInstallGeneratesExpectedWorkspaceArtifacts(): void
    {
        $generated = self::$workspace->installedPackageRoot . '/src/generated';
        foreach ([
            'index.php',
            'functions.php',
            'macro.functions.php',
            'Example.php',
            'cdata/Example.php',
            'scalar/Example.php',
            'cdata/scalar/Example.php',
            'ExampleManifest.php',
            'ExampleContext.php',
            'ExampleException.php',
            'const.php',
            'function.aliases.php',
            'example.ffi.php',
            'example.bridge.rs',
        ] as $file) {
            self::assertFileExists($generated . '/' . $file);
        }

        // The self-contained type layer ships with the SDK (one class per file)
        // and is copied into @pnlx/runtime with the rest of the runtime.
        $helpers = self::$workspace->projectRoot . '/@pnlx/runtime/Pnlx/Helpers';
        self::assertFileExists($helpers . '/AbstractInteger.php');
        self::assertFileExists($helpers . '/UnsignedInt64.php');
        // The wrapper-aware is_*/gettype helpers live alongside is_null in Util.
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/runtime/Pnlx/Util/functions.php');

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
            'macro.functions.php',
            'Example.php',
            'cdata/Example.php',
            'scalar/Example.php',
            'cdata/scalar/Example.php',
            'ExampleManifest.php',
            'ExampleContext.php',
            'ExampleException.php',
            'const.php',
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
                if (!is_dir(dirname($goldenPath))) {
                    mkdir(dirname($goldenPath), 0o777, true);
                }
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
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        // The entity is pure static; returns are wrapped value objects.
        $version = $cls::example_version();
        self::assertInstanceOf(\Pnlx\Helpers\String_::class, $version);
        self::assertSame('1.2.3', (string) $version);

        // A dynamic (variable) method name dispatches identically.
        $fn = 'example_version';
        $dynamicVersion = $cls::$fn();
        self::assertInstanceOf(\Pnlx\Helpers\String_::class, $dynamicVersion);
        self::assertSame('1.2.3', (string) $dynamicVersion);

        // Parameters accept a plain PHP int; returns come back wrapped.
        $sum = $cls::example_add(2, 3);
        self::assertInstanceOf(\Pnlx\Helpers\AnySizeInteger::class, $sum);
        self::assertSame(5, $sum->toInt());

        // The generated camelCase alias is also a static method.
        $aliased = $cls::exampleAdd(2, 3);
        self::assertInstanceOf(\Pnlx\Helpers\AnySizeInteger::class, $aliased);
        self::assertSame(5, $aliased->toInt());

        // A wrapped integer can be passed straight back in as an argument.
        $wrapped = $cls::example_add(new \Pnlx\Helpers\Int_(2), new \Pnlx\Helpers\Int_(3));
        self::assertInstanceOf(\Pnlx\Helpers\AnySizeInteger::class, $wrapped);
        self::assertSame(5, $wrapped->toInt());
    }

    public function testFunctionLikeMacrosBecomePhpFunctions(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        // EXAMPLE_TWICE(N) -> example_add(N, N), delegating to the static entity.
        self::assertTrue(function_exists('Pnlx\\Func\\Example\\EXAMPLE_TWICE'));
        $twice = \Pnlx\Func\Example\EXAMPLE_TWICE(21);
        self::assertInstanceOf(\Pnlx\Helpers\AnySizeInteger::class, $twice);
        self::assertSame(42, $twice->toInt());

        // EXAMPLE_MISSING(X) calls a C function this library does not define, so it
        // was generated as a thrower.
        self::assertTrue(function_exists('Pnlx\\Func\\Example\\EXAMPLE_MISSING'));
        $this->expectException(\Pnlx\Exception\PHPNativeLibraryException::class);
        \Pnlx\Func\Example\EXAMPLE_MISSING(1);
    }

    public function testRuntimeCanExposeGeneratedGlobalFunctions(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        self::assertTrue(Runtime::enableFunctions(self::$workspace->projectRoot));
        // Generated global functions live under the \Pnlx\Func\<Class> namespace.
        self::assertFalse(function_exists('example_add'));
        self::assertTrue(function_exists('Pnlx\\Func\\Example\\example_add'));
        $fnResult = \Pnlx\Func\Example\example_add(2, 3);
        self::assertInstanceOf(\Pnlx\Helpers\AnySizeInteger::class, $fnResult);
        self::assertSame(5, $fnResult->toInt());
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
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        self::assertTrue($GLOBALS['pnlx_test_preload_ran'] ?? false);
        self::assertTrue($GLOBALS['pnlx_test_postload_ran'] ?? false);
        // The entity was already booted by the time both hooks ran.
        self::assertTrue($GLOBALS['pnlx_test_preload_entity_booted'] ?? false);
        self::assertTrue($GLOBALS['pnlx_test_postload_entity_booted'] ?? false);
    }

    public function testRuntimeReturnsBridgeInfoByClass(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $info = $runtime->loadManifest(self::EXAMPLE_CLASS);

        self::assertSame('example/example', $info->name());
        self::assertSame('1.2.3', $info->version());
        self::assertSame(hash_file('sha256', self::$workspace->bridgeLibraryPath), $info->hash());
        self::assertSame(self::$workspace->bridgeLibraryPath, $info->path());

        // The same metadata is published onto the entity's static properties when
        // it boots (filled directly, since there is no __getStatic).
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $entity = new \ReflectionClass(self::exampleClass());
        self::assertSame('example/example', $entity->getStaticPropertyValue('name'));
        self::assertSame('1.2.3', $entity->getStaticPropertyValue('version'));
        self::assertSame(self::$workspace->bridgeLibraryPath, $entity->getStaticPropertyValue('path'));
        self::assertSame(
            hash_file('sha256', self::$workspace->bridgeLibraryPath),
            $entity->getStaticPropertyValue('hash')
        );
    }

    public function testGeneratedEntityShapeIsOverrideFriendly(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $reflection = new \ReflectionClass(self::exampleClass());

        self::assertFalse($reflection->isFinal());
        // The only inherited named surface is the magic __callStatic (a C function
        // can never be named that); there is no __get/__call to collide.
        self::assertTrue($reflection->hasMethod('__callStatic'));
        self::assertFalse($reflection->hasMethod('__get'));
        self::assertFalse($reflection->hasMethod('__call'));
        // Generated methods are static.
        self::assertTrue($reflection->getMethod('example_version')->isStatic());
        self::assertTrue($reflection->getMethod('exampleAdd')->isStatic());

        // Metadata is exposed as static properties redeclared on the class, so it
        // never collides with a generated method named after a C function.
        foreach (['name', 'version', 'hash', 'description', 'path'] as $field) {
            self::assertTrue($reflection->hasProperty($field));
            self::assertTrue($reflection->getProperty($field)->isStatic());
        }

        // The class carries the native-library attributes …
        $libNameAttrs = $reflection->getAttributes(\Pnlx\Attribute\NativeLibraryName::class);
        self::assertCount(1, $libNameAttrs);
        $libName = $libNameAttrs[0]->newInstance();
        self::assertInstanceOf(\Pnlx\Attribute\NativeLibraryName::class, $libName);
        self::assertSame('example/example', $libName->name);
        self::assertCount(1, $reflection->getAttributes(\Pnlx\Attribute\AutoGeneratedByPnlx::class));

        // … and each generated method records the raw C symbol it wraps.
        $rawAttrs = $reflection->getMethod('exampleAdd')->getAttributes(\Pnlx\Attribute\RawNativeName::class);
        self::assertCount(1, $rawAttrs);
        $raw = $rawAttrs[0]->newInstance();
        self::assertInstanceOf(\Pnlx\Attribute\RawNativeName::class, $raw);
        self::assertSame('example_add', $raw->name);
    }

    public function testRuntimeProvidesAllocatorAndStaticUtil(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $buffer = $runtime->allocator()->voidPointerArray(1);

        self::assertInstanceOf(\FFI\CData::class, $buffer);
        // Typed allocation via FFI::new returns FFI\CData.
        self::assertInstanceOf(
            \FFI\CData::class,
            $runtime->allocator()->make(\Pnlx\FFI\AllocationType::Int64)
        );
        self::assertSame('int64_t', \Pnlx\FFI\AllocationType::Int64->cType());
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
