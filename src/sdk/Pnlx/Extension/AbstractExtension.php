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
 * call boots the extension once (opens the native library) and then dispatches
 * the requested C function.
 *
 * Collision-safety is the whole point of this shape: the ONLY named surface the
 * generated subclass inherits is the magic {@see __callStatic()} (a C function
 * can never be named `__callStatic`). Everything else lives in `private` methods
 * reached through `self::` (not inherited, never overridden) or in fully-qualified
 * external helpers (`\Pnlx\FFI\ArgumentMarshaller::…`). So a C function named
 * `boot`, `dispatch`, `name`, … can never clash with the runtime.
 *
 * Metadata is install-time information, baked into the generated subclass as
 * constants (`Libsdl::NAME`, `VERSION`, `HASH`, `DESCRIPTION`, `PATH`) when the
 * package is installed — not read back from the manifest/pathmap at runtime.
 */
abstract class AbstractExtension
{
    /** The package's generated FFI cdef file; the subclass overrides it. */
    protected const string FFI_FILE = '';

    /** The generated alias-map file (PHP name -> native symbol), a cdef sibling. */
    protected const string ALIASES_FILE = 'function.aliases.php';

    /** Per-generated-class boot sentinel; subclasses override with a hard-to-collide token. */
    protected const string PNLX_BOOT_TOKEN = '';

    public const string NAME = '';

    public const string VERSION = '';

    public const string HASH = '';

    public const string DESCRIPTION = '';

    public const string PATH = '';

    /**
     * Loaded native library per concrete class.
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
     * The generated static methods also route their native dispatch through here
     * (`static::__callStatic('SDL_Init', [...])`) because the name `__callStatic`
     * can never collide with a C function.
     *
     * @param list<mixed> $arguments
     */
    public static function __callStatic(string $name, array $arguments): mixed
    {
        if (static::PNLX_BOOT_TOKEN !== '' && $name === static::PNLX_BOOT_TOKEN) {
            if (!isset(self::$initialized[static::class])) {
                self::boot();
            }

            return null;
        }

        if (!isset(self::$initialized[static::class])) {
            self::boot();
        }

        return self::$natives[static::class]->call($name, $arguments);
    }

    /**
     * The booted {@see NativeLibrary} for this extension, used SDK-internally to
     * resolve exported globals ({@see \Pnlx\FFI\ArgumentMarshaller}) and allocate
     * package structs (`new ...\Types\<struct>()`). Examples never call this
     * directly. Named with a `pnlx` prefix so it can't realistically clash with a
     * C function (and a data symbol can't share a function symbol's name anyway).
     */
    public static function pnlxNativeLibrary(): NativeLibrary
    {
        if (!isset(self::$initialized[static::class])) {
            self::boot();
        }

        return self::$natives[static::class];
    }

    /**
     * One-time per-class setup: verify the baked native library against its constant hash
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

        if (is_file(static::PATH) && static::HASH !== '') {
            $actual = hash_file('sha256', static::PATH);
            if ($actual === false || !hash_equals(static::HASH, $actual)) {
                throw new ExtensionLoadException('Native library hash does not match the generated constant.');
            }
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
            false,
        );

        self::$initialized[static::class] = true;
    }
}
