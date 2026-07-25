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
            'ExampleLibraryComponent.php',
            'cdata/ExampleLibraryComponent.php',
            'scalar/ExampleLibraryComponent.php',
            'cdata/scalar/ExampleLibraryComponent.php',
            'ExampleManifest.php',
            'ExampleContext.php',
            'ExampleException.php',
            'const.php',
            'scalar/const.php',
            'function.aliases.php',
            'example.ffi.php',
        ] as $file) {
            self::assertFileExists($generated . '/' . $file);
        }

        // The self-contained type layer ships with the SDK (one class per file)
        // and is copied into @pnlx/runtime with the rest of the runtime.
        $helpers = self::$workspace->projectRoot . '/@pnlx/runtime/Pnlx/Types';
        self::assertFileExists($helpers . '/AbstractInteger.php');
        self::assertFileExists($helpers . '/UnsignedInt64.php');
        // The wrapper-aware is_*/gettype helpers live alongside is_null in Util.
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/runtime/Pnlx/Util/functions.php');

        // Global helpers are generated under the \Pnlx\Func\<Class> namespace.
        $functions = (string) file_get_contents($generated . '/functions.php');
        self::assertStringContainsString('namespace Pnlx\\Func\\Example;', $functions);
        self::assertStringContainsString("function_exists('Pnlx\\\\Func\\\\Example\\\\example_add')", $functions);

        // The workspace itself contains the lock and pathmap.
        self::assertFileExists(self::$workspace->projectRoot . '/@pnlx/pnlx-pathmap.json');
    }

    public function testNativeReflectionHelpersDetectGeneratedClassesAndFunctions(): void
    {
        require_once self::$workspace->projectRoot . '/@pnlx/autoload.php';

        self::assertTrue(\Pnlx\Util\is_native_class(self::exampleClass()));
        self::assertTrue(\Pnlx\Util\is_native_function([self::exampleClass(), 'example_add']));
        self::assertTrue(\Pnlx\Util\is_native_function(self::exampleClass() . '::example_add'));
        self::assertTrue(\Pnlx\Util\is_native_function('Pnlx\\Func\\Example\\example_add'));

        self::assertFalse(\Pnlx\Util\is_native_class(self::class));
        self::assertFalse(\Pnlx\Util\is_native_function('strlen'));
        self::assertFalse(\Pnlx\Util\is_native_function('Pnlx\\Func\\Example\\missing'));
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

        $requires = $extension['native_libraries'];
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
            'ExampleLibraryComponent.php',
            'cdata/ExampleLibraryComponent.php',
            'scalar/ExampleLibraryComponent.php',
            'cdata/scalar/ExampleLibraryComponent.php',
            'ExampleManifest.php',
            'ExampleContext.php',
            'ExampleException.php',
            'const.php',
            'scalar/const.php',
            'function.aliases.php',
            'example.ffi.php',
            'enums/example_mode.php',
            'types/example_number.php',
            'types/example_opaque.php',
            'types/example_point.php',
            'types/example_value.php',
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

        // The native library path (absolute, in a random temp workspace) and hash
        // ride on the #[NativeLibrary] attribute and are machine/run-specific, so
        // blank their argument values.
        $normalized = preg_replace(
            "/((?:path|hash): ')[^']*(')/",
            '${1}<normalized>${2}',
            $normalized ?? $content
        );

        return $normalized ?? $content;
    }

    public function testRuntimeLoadsExampleThroughDynamicFfi(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        // The entity is pure static; returns are wrapped value objects.
        $version = $cls::example_version();
        self::assertInstanceOf(\Pnlx\Types\String_::class, $version);
        self::assertSame('1.2.3', (string) $version);

        // A dynamic (variable) method name dispatches identically.
        $fn = 'example_version';
        $dynamicVersion = $cls::$fn();
        self::assertInstanceOf(\Pnlx\Types\String_::class, $dynamicVersion);
        self::assertSame('1.2.3', (string) $dynamicVersion);

        // Parameters accept a plain PHP int; returns come back wrapped.
        $sum = $cls::example_add(2, 3);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $sum);
        self::assertSame(5, $sum->toInt());

        // The generated camelCase alias is also a static method.
        $aliased = $cls::exampleAdd(2, 3);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $aliased);
        self::assertSame(5, $aliased->toInt());

        // A wrapped integer can be passed straight back in as an argument.
        $wrapped = $cls::example_add(new \Pnlx\Types\Int_(2), new \Pnlx\Types\Int_(3));
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $wrapped);
        self::assertSame(5, $wrapped->toInt());
    }

    public function testRuntimePassesPhpCallableAsCCallback(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        // A C function-pointer parameter is generated as a `callable` argument. The
        // PHP closure is passed straight through to PHP FFI, which builds a C callback
        // trampoline; the native example_apply invokes it synchronously and returns
        // its result plus one (20 * 2 + 1).
        $result = $cls::example_apply(20, static fn (int $value): int => $value * 2);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $result);
        self::assertSame(41, $result->toInt());

        // A plain callable (a named function) is accepted by the `callable` hint too.
        $result = $cls::example_apply(5, 'Pnlx\\Tests\\example_callback_increment');
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $result);
        self::assertSame(7, $result->toInt());
    }

    public function testRuntimeMapsCEnumToPhpEnum(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        // The C `enum example_mode` is generated as an int-backed PHP enum.
        self::assertTrue(enum_exists('Pnlx\\Example\\Enums\\example_mode'));

        // A raw int is accepted; the int the C function returns is mapped back to the
        // enum (OFF=0 -> ON=1).
        $on = $cls::example_next_mode(0);
        self::assertInstanceOf(\BackedEnum::class, $on);
        self::assertSame(1, $on->value);
        self::assertSame('Pnlx\\Example\\Enums\\example_mode', $on::class);

        // Passing a PHP enum case back in sends its backing int (ON=1 -> AUTO=10).
        $auto = $cls::example_next_mode($on);
        self::assertInstanceOf(\BackedEnum::class, $auto);
        self::assertSame(10, $auto->value);

        // AUTO(10) wraps around to OFF(0).
        $off = $cls::example_next_mode($auto);
        self::assertInstanceOf(\BackedEnum::class, $off);
        self::assertSame(0, $off->value);
    }

    public function testRuntimeStructFieldAccessors(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        // Allocate a struct from PHP and set its fields with the typed (chainable)
        // setters, then pass the wrapper where the C API wants `example_point *`.
        // (The generated wrapper is on a temp dir at test time; stubs/example-fixture.php
        // declares its shape for PHPStan, so the natural typed API analyses cleanly.)
        $point = new \Pnlx\Example\Types\example_point();
        $point->setX(3)->setY(4);
        self::assertSame(3, $point->getX());
        self::assertSame(4, $point->getY());

        $sum = $cls::example_point_sum($point);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $sum);
        self::assertSame(7, $sum->toInt());

        // A C function that writes through the struct is read back via the getters.
        $cls::example_point_init($point, 10, 20);
        self::assertSame(10, $point->getX());
        self::assertSame(20, $point->getY());
    }

    public function testRuntimeNestedUnionAccessors(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        $number = new \Pnlx\Example\Types\example_number();
        $number->setInteger(41);
        $value = new \Pnlx\Example\Types\example_value();
        $value->setKind(1)->setNumber($number);

        self::assertSame(1, $value->getKind());
        self::assertSame(41, $value->getNumber()->getInteger());
        $integer = $cls::example_value_integer($value);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $integer);
        self::assertSame(41, $integer->toInt());

        $cls::example_value_init($value, 73);
        self::assertSame(73, $value->getNumber()->getInteger());
    }

    public function testRuntimeOpaqueAggregateFallback(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;

        $value = new \Pnlx\Example\Types\example_opaque();
        $cls::example_opaque_write($value, 1234);

        $word = $cls::example_opaque_read($value);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $word);
        self::assertSame(1234, $word->toInt());
    }

    public function testAllocatorAllocatesInTheExtensionScope(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        $allocator = \Pnlx\FFI\NativeLibraryRegistry::allocator(self::exampleClass());
        self::assertInstanceOf(\FFI\CData::class, $allocator->new('struct example_point'));
        self::assertInstanceOf(\FFI\CData::class, $allocator->cString('hello'));
    }

    public function testAllocationScopePinsThenReleases(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        $scope = \Pnlx\FFI\NativeLibraryRegistry::allocator(self::exampleClass())->scope();
        $scope->new('struct example_point');
        $scope->cString('hi');
        self::assertSame(2, $scope->count());

        // After release the backing memory is no longer pinned, so the scope refuses
        // to hand out values whose lifetime it can no longer manage.
        $scope->release();
        self::assertSame(0, $scope->count());
        $this->expectException(\Pnlx\Exception\PHPNativeLibraryException::class);
        $scope->new('struct example_point');
    }

    public function testUnexportedDeclaredFunctionIsFilteredOut(): void
    {
        // example_unexported is declared in example.h but never defined in the native
        // library, so it is not in its export table. The export-symbol filter (which
        // parses the binary directly, not via `nm`) must drop it — otherwise
        // FFI::cdef fails to resolve it and breaks the whole extension (the SDL_main
        // scenario reported on a host without binutils).
        $cdef = file_get_contents(
            self::$workspace->installedPackageRoot . '/src/generated/example.ffi.php'
        );
        self::assertIsString($cdef);
        self::assertStringNotContainsString('example_unexported', $cdef);

        // The extension still loads and a real export works.
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $cls = self::EXAMPLE_CLASS;
        $sum = $cls::example_add(2, 3);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $sum);
        self::assertSame(5, $sum->toInt());
    }

    public function testFunctionLikeMacrosBecomePhpFunctions(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);

        // EXAMPLE_TWICE(N) -> example_add(N, N), delegating to the static entity.
        self::assertTrue(function_exists('Pnlx\\Func\\Example\\EXAMPLE_TWICE'));
        $twice = \Pnlx\Func\Example\EXAMPLE_TWICE(21);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $twice);
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
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $fnResult);
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

    public function testRuntimeReturnsNativeLibraryInfoByClass(): void
    {
        $runtime = new Runtime(self::$workspace->projectRoot);
        $info = $runtime->loadManifest(self::EXAMPLE_CLASS);

        self::assertSame('example/example', $info->name());
        self::assertSame('1.2.3', $info->version());
        self::assertSame(hash_file('sha256', self::$workspace->nativeLibraryPath), $info->hash());
        self::assertSame(self::$workspace->nativeLibraryPath, $info->path());

        // The same metadata rides on the entity's #[NativeLibrary] attribute
        // (path/hash stamped in after the native library was resolved).
        $runtime->loadEntrypoint(self::EXAMPLE_CLASS);
        $attributes = (new \ReflectionClass(self::exampleClass()))
            ->getAttributes(\Pnlx\Attribute\NativeLibrary::class);
        self::assertCount(1, $attributes);
        $lib = $attributes[0]->newInstance();
        self::assertSame('example/example', $lib->name);
        self::assertSame('1.2.3', $lib->version);
        self::assertSame(self::$workspace->nativeLibraryPath, $lib->path);
        self::assertSame(
            hash_file('sha256', self::$workspace->nativeLibraryPath),
            $lib->hash
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

        // The method group is mixed in from the generated Component trait.
        self::assertContains(
            'Pnlx\\Example\\ExampleLibraryComponent',
            class_uses(self::exampleClass()) ?: []
        );

        // Metadata rides on the #[NativeLibrary] attribute, not constants, so a
        // composed class can mix in several Components without colliding metadata.
        $libAttrs = $reflection->getAttributes(\Pnlx\Attribute\NativeLibrary::class);
        self::assertCount(1, $libAttrs);
        $lib = $libAttrs[0]->newInstance();
        self::assertInstanceOf(\Pnlx\Attribute\NativeLibrary::class, $lib);
        self::assertSame('example/example', $lib->name);
        self::assertSame('1.2.3', $lib->version);
        self::assertCount(1, $reflection->getAttributes(\Pnlx\Attribute\AutoGeneratedByPnlx::class));

        // … and each generated method records the raw C symbol it wraps.
        $rawAttrs = $reflection->getMethod('exampleAdd')->getAttributes(\Pnlx\Attribute\RawNativeName::class);
        self::assertCount(1, $rawAttrs);
        $raw = $rawAttrs[0]->newInstance();
        self::assertInstanceOf(\Pnlx\Attribute\RawNativeName::class, $raw);
        self::assertSame('example_add', $raw->name);
    }

    public function testComposeExposesBothMembersThroughOneSharedScope(): void
    {
        require_once self::$workspace->projectRoot . '/@pnlx/autoload.php';

        $composite = ltrim(RuntimeWorkspace::COMPOSITE_CLASS, '\\');
        if (!class_exists($composite)) {
            self::fail("The composite {$composite} was not generated by `pnl compose`.");
        }

        // example_add comes from the `example` member, extra_triple from `extra`,
        // and they run through the one shared FFI scope the composite boots.
        $sum = $composite::example_add(2, 3);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $sum);
        self::assertSame(5, $sum->toInt());

        $tripled = $composite::extra_triple(4);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $tripled);
        self::assertSame(12, $tripled->toInt());

        // A struct pointer must round-trip: it is allocated through a member's type
        // wrapper (Example::pnlxNativeLibrary()) yet filled and read through the
        // composite — so the member and composite must share one FFI scope, or FFI
        // rejects the pointer as foreign.
        $point = new \Pnlx\Example\Types\example_point();
        $composite::example_point_init($point, 3, 4);
        $pointSum = $composite::example_point_sum($point);
        self::assertInstanceOf(\Pnlx\Types\AnySizeInteger::class, $pointSum);
        self::assertSame(7, $pointSum->toInt());
    }

    public function testComposedGlobalFunctionsDelegateToTheComposite(): void
    {
        require_once self::$workspace->projectRoot . '/@pnlx/autoload.php';

        // With global_functions on, `pnl compose` also emits \Pnlx\Func\Demo\* global
        // functions (from both members) that delegate to the composite class, so
        // they share its one FFI scope. They carry the AutoGeneratedByPnlx attribute.
        self::assertTrue(function_exists('Pnlx\\Func\\Demo\\example_add'));
        self::assertTrue(function_exists('Pnlx\\Func\\Demo\\extra_triple'));
        self::assertTrue(\Pnlx\Util\is_native_function('Pnlx\\Func\\Demo\\example_add'));
        self::assertTrue(\Pnlx\Util\is_native_function('Pnlx\\Func\\Demo\\extra_triple'));
    }

    public function testStaticUtilHelpers(): void
    {
        self::assertSame('ok', \Pnlx\Util::cString('ok'));
        // \Pnlx\Util\is_null() falls back to PHP's is_null for non-CData values.
        self::assertTrue(\Pnlx\Util\is_null(null));
        self::assertFalse(\Pnlx\Util\is_null('not cdata'));
    }

    /**
     * Every generated `char *` return is routed through cStringOrNull(), so a C
     * function returning NULL (getenv() of an unset name, strstr() with no match,
     * idn2_check_version() below the requested version) surfaces as PHP null
     * instead of dereferencing the null pointer and fataling.
     */
    public function testCStringOrNullMapsNullPointerToNull(): void
    {
        $scope = \FFI::cdef('');

        // A live `char *` reads back as the string.
        $buffer = $scope->new('char[3]');
        if (!$buffer instanceof \FFI\CData) {
            self::fail('Unable to allocate a char buffer.');
        }
        \FFI::memcpy($buffer, "hi\0", 3);
        $livePtr = $scope->cast('char *', \FFI::addr($buffer));
        self::assertSame('hi', \Pnlx\Util::cStringOrNull($livePtr));

        // A NULL `char *` becomes null rather than a fatal FFI::string(NULL).
        $nullPtr = $scope->new('char *');
        self::assertNull(\Pnlx\Util::cStringOrNull($nullPtr));

        // A plain PHP string passes through; PHP null stays null.
        self::assertSame('ok', \Pnlx\Util::cStringOrNull('ok'));
        self::assertNull(\Pnlx\Util::cStringOrNull(null));
    }

    public function testInstallRecordsNativeLibraryInPathmap(): void
    {
        $pathmap = self::$workspace->pathmap();
        $requires = $pathmap['native_libraries'] ?? null;
        self::assertIsArray($requires);
        $native = $requires['example'] ?? null;
        self::assertIsArray($native);
        self::assertSame(self::$workspace->nativeLibraryPath, $native['path']);
        self::assertSame(hash_file('sha256', self::$workspace->nativeLibraryPath), $native['sha256']);
    }
}

/**
 * A named callable target for {@see RuntimeTest::testRuntimePassesPhpCallableAsCCallback()};
 * proves a plain function name (not only a closure) satisfies the generated
 * `callable` parameter and reaches the native callback.
 */
function example_callback_increment(int $value): int
{
    return $value + 1;
}
