<?php

declare(strict_types=1);

namespace Pnlx\Extension;

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\NativeLibrary;
use Pnlx\Runtime;

/**
 * Static base shared by every generated extension entity.
 *
 * A C library is a bag of functions, not an object, so entities are never
 * instantiated: call them statically, `Libsdl::SDL_Init(...)`. The first static
 * call boots the extension once (opens the native bridge) and returns without
 * dispatching; the generated file calls `<Class>::initialize()` once at the very
 * bottom of its definition to absorb that bootstrap, so every later call goes
 * straight to the native library.
 *
 * Collision-safety is the whole point of this shape: the ONLY named surface the
 * generated subclass inherits is the magic {@see __callStatic()} (a C function
 * can never be named `__callStatic`). Everything else lives in `private` methods
 * reached through `self::` (not inherited, never overridden) or in fully-qualified
 * external helpers (`\Pnlx\FFI\ArgumentMarshaller::…`). So a C function named
 * `boot`, `dispatch`, `name`, … can never clash with the runtime.
 *
 * Metadata is *build-time* information, baked into the generated subclass as
 * constants (`Libsdl::NAME`, `VERSION`, `HASH`, `DESCRIPTION`, `PATH`) when the
 * package is installed — not read back from the manifest/pathmap at runtime. The
 * `HASH`/`PATH` of the compiled bridge are stamped in after it is built.
 */
abstract class AbstractExtension
{
    /** The package's generated FFI cdef file; the subclass overrides it. */
    protected const string FFI_FILE = '';

    /** The generated alias-map file (PHP name -> bridge symbol), a cdef sibling. */
    protected const string ALIASES_FILE = 'function.aliases.php';

    public const string NAME = '';

    public const string VERSION = '';

    public const string HASH = '';

    public const string DESCRIPTION = '';

    public const string PATH = '';

    /**
     * Compiled native library per concrete class.
     *
     * @var array<class-string, NativeLibrary>
     */
    private static array $natives = [];

    /**
     * Boot guard per concrete class.
     *
     * @var array<class-string, true>
     */
    private static array $initialized = [];

    /** Entities are pure static; instantiating one is forbidden. */
    private function __construct()
    {
    }

    /**
     * The first static call on a class boots it and returns without dispatching;
     * the generated file's bottom-of-class `initialize()` call absorbs that, so a
     * later C function of any name (including `initialize`) dispatches normally.
     *
     * The generated static methods also route their native dispatch through here
     * (`static::__callStatic('SDL_Init', [...])`) because the name `__callStatic`
     * can never collide with a C function.
     *
     * @param list<mixed> $arguments
     */
    public static function __callStatic(string $name, array $arguments): mixed
    {
        if (!isset(self::$initialized[static::class])) {
            self::boot();

            return null;
        }

        return self::$natives[static::class]->call($name, $arguments);
    }

    /**
     * One-time per-class setup: verify the baked bridge against its constant hash
     * and open it. The cdef and alias map are siblings of the generated entity, so
     * we locate them from this class's own file — no manifest or pathmap lookup.
     *
     * Private and reached only via `self::boot()`, so it is never inherited and a
     * C function named `boot` cannot interfere.
     */
    private static function boot(): void
    {
        $runtime = new Runtime();

        // Resolve the scalar-args feature here, where the project root is known,
        // and hand it to the (collision-safe, fully-qualified) marshaller.
        \Pnlx\FFI\ArgumentMarshaller::rememberScalarsAllowed(
            static::class,
            Runtime::useScalarsInParams($runtime->projectRoot()),
        );

        if (!is_file(static::PATH)) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }
        $actual = hash_file('sha256', static::PATH);
        if ($actual === false || !hash_equals(static::HASH, $actual)) {
            throw new ExtensionLoadException('Native bridge hash does not match the generated constant.');
        }

        // Entity variants live in cdata/scalar subdirs; the cdef/alias map sit in
        // the base generated dir, so walk up from this file until the cdef appears.
        $directory = dirname((string) (new \ReflectionClass(static::class))->getFileName());
        while (!is_file($directory . '/' . static::FFI_FILE) && dirname($directory) !== $directory) {
            $directory = dirname($directory);
        }

        self::$natives[static::class] = NativeLibrary::load(
            $directory . '/' . static::FFI_FILE,
            static::PATH,
            $directory . '/' . static::ALIASES_FILE,
        );

        self::$initialized[static::class] = true;
    }
}
