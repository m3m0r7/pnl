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
 * call boots the extension once (loads the native bridge, fills the metadata
 * properties) and returns without dispatching; the generated file calls
 * `<Class>::initialize()` once at the very bottom of its definition to absorb
 * that bootstrap, so every later call goes straight to the native library.
 *
 * Collision-safety is the whole point of this shape: the ONLY named surface the
 * generated subclass inherits is the magic {@see __callStatic()} (a C function
 * can never be named `__callStatic`). Everything else lives in `private`
 * methods reached through `self::` (not inherited, never overridden) or in
 * fully-qualified external helpers (`\Pnlx\FFI\ArgumentMarshaller::…`). So a C
 * function named `boot`, `dispatch`, `name`, … can never clash with the runtime.
 *
 * Metadata is exposed as static properties the generated subclass redeclares
 * (so each class gets its own storage): `Libsdl::$name`, `$version`, `$hash`,
 * `$description`, `$path`.
 */
abstract class AbstractExtension
{
    /** The package's generated FFI cdef file; each subclass overrides it. */
    protected const string FFI_FILE = '';

    public static string $name;

    public static string $version;

    public static string $hash;

    public static string $description;

    public static string $path;

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
     * One-time per-class setup: resolve the runtime, load the manifest and native
     * bridge, and publish the metadata into the static properties.
     *
     * Private and reached only via `self::boot()`, so it is never inherited and a
     * C function named `boot` cannot interfere. `self::` keeps late static binding
     * pointed at the concrete class, so `static::$name` targets its own property.
     */
    private static function boot(): void
    {
        $runtime = new Runtime();
        $manifest = $runtime->loadManifest(static::class);
        if (!is_file($manifest->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }

        self::$natives[static::class] = $runtime->native(static::class, static::FFI_FILE);

        static::$name = $manifest->name();
        static::$version = $manifest->version();
        static::$hash = $manifest->hash();
        static::$description = $manifest->description();
        static::$path = $manifest->path();

        self::$initialized[static::class] = true;
    }
}
