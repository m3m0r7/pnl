<?php

declare(strict_types=1);

namespace Pnlx\Extension;

use Pnlx\FFI\NativeLibraryRegistry;

/**
 * Static base shared by every generated extension entity.
 *
 * A C library is a bag of functions, not an object, so entities are never
 * instantiated: call them statically, `Libsdl::SDL_Init(...)`. An entity (and its
 * generated `<Class>LibraryComponent` trait) carries ONLY the methods named after C
 * functions — nothing else lives here, so composing several Components into one
 * class can only ever clash on real C-function names, never on runtime plumbing.
 *
 * The only inherited surface is the two magic methods, which a C function can never
 * be named: `__construct` (instantiation is forbidden) and `__callStatic`, which
 * forwards every native dispatch to {@see NativeLibraryRegistry}. All the machinery
 * — booting, the loaded library, install metadata — lives in that registry, keyed
 * by class, never as a method on the entity.
 */
abstract class AbstractExtension
{
    /** Entities are pure static; instantiating one is forbidden. */
    private function __construct()
    {
    }

    /**
     * Forward a native dispatch to the registry. Generated methods route here
     * (`static::__callStatic('SDL_Init', [...])`) because the name `__callStatic`
     * can never collide with a C function.
     *
     * @param list<mixed> $arguments
     */
    public static function __callStatic(string $name, array $arguments): mixed
    {
        return NativeLibraryRegistry::call(static::class, $name, $arguments);
    }
}
