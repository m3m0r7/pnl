<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use Pnlx\Attribute\NativeLibrary as NativeLibraryInfo;
use Pnlx\Attribute\NativeLibraryComponent;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Runtime;

/**
 * The runtime machinery behind every generated extension, kept OFF the entity so
 * an entity (and its `<Class>LibraryComponent` trait) exposes nothing but the C
 * functions it wraps — no named helper that could clash with a C symbol, and
 * nothing extra to collide when several Components are composed into one class.
 * Entities reach it only through the magic `__callStatic` (which a C function can
 * never be named); generated method bodies and type wrappers call it directly.
 *
 * It keeps one loaded {@see NativeLibrary} per concrete entity class. A class that
 * mixes in a single Component binds that one library (hash-verified); a class that
 * mixes in several merges their cdefs and co-loads their libraries into ONE shared
 * scope ({@see CdefComposer}/{@see NativeLibrary::composite()}), and that shared
 * scope is also shared with each member entity so a CData created via a member's
 * type wrapper stays in the one FFI world the composite calls through.
 */
final class NativeLibraryRegistry
{
    /** @var array<class-string, NativeLibrary> */
    private static array $natives = [];

    /** @var array<class-string, true> */
    private static array $initialized = [];

    /**
     * Dispatch a native call for an entity class, booting it on first use.
     *
     * @param class-string $class
     * @param list<mixed>  $arguments
     */
    public static function call(string $class, string $name, array $arguments): mixed
    {
        return self::of($class)->call($name, $arguments);
    }

    /**
     * The booted {@see NativeLibrary} for an entity class (used by generated type
     * wrappers to allocate/reinterpret and by {@see ArgumentMarshaller} for globals).
     *
     * @param class-string $class
     */
    public static function of(string $class): NativeLibrary
    {
        self::boot($class);

        return self::$natives[$class];
    }

    /**
     * Ensure an entity class's native library is loaded (idempotent). Generated
     * methods call this before marshalling arguments, which needs the booted scope.
     *
     * @param class-string $class
     */
    public static function boot(string $class): void
    {
        if (!isset(self::$initialized[$class])) {
            self::load($class);
        }
    }

    /**
     * An entity class's install-time metadata from its `#[NativeLibrary]` attribute.
     *
     * @param class-string $class
     */
    public static function info(string $class): NativeLibraryInfo
    {
        $attributes = (new \ReflectionClass($class))->getAttributes(NativeLibraryInfo::class);
        if ($attributes === []) {
            throw new ExtensionLoadException(sprintf('%s is missing its #[NativeLibrary] metadata attribute.', $class));
        }

        return $attributes[0]->newInstance();
    }

    /**
     * One-time per-class setup: reflect over the class's Component traits and bind
     * (single) or merge + co-load (several) their native libraries.
     *
     * @param class-string $class
     */
    private static function load(string $class): void
    {
        $scalarsAllowed = Runtime::useScalarsInParams((new Runtime())->projectRoot());
        ArgumentMarshaller::rememberScalarsAllowed($class, $scalarsAllowed);

        $origins = self::componentOrigins($class);
        if ($origins === []) {
            // A class carrying its own #[NativeLibrary] without a Component trait.
            $origins = [$class];
        }
        $descriptors = array_map(self::libraryMetadata(...), $origins);

        if (count($descriptors) === 1) {
            $descriptor = $descriptors[0];
            if (is_file($descriptor['path']) && $descriptor['hash'] !== '') {
                $actual = hash_file('sha256', $descriptor['path']);
                if ($actual === false || !hash_equals($descriptor['hash'], $actual)) {
                    throw new ExtensionLoadException('Native library hash does not match the generated metadata.');
                }
            }

            self::$natives[$class] = NativeLibrary::load(
                $descriptor['cdef'],
                $descriptor['path'],
                $descriptor['aliases'],
                false,
                $descriptor['libraries'],
            );
            self::$initialized[$class] = true;

            return;
        }

        // Multiple components: merge their cdefs and co-load every library into one
        // shared scope (in-memory, so we reuse the public composite loader).
        $cdefs = [];
        $aliases = [];
        $libraries = [];
        foreach ($descriptors as $descriptor) {
            $cdef = require $descriptor['cdef'];
            if (!is_string($cdef)) {
                throw new ExtensionLoadException(sprintf('CDEF file %s must return a string.', $descriptor['cdef']));
            }
            $cdefs[] = $cdef;

            $memberAliases = require $descriptor['aliases'];
            if (!is_array($memberAliases)) {
                throw new ExtensionLoadException(sprintf('Aliases file %s must return an array.', $descriptor['aliases']));
            }
            /** @var array<string, string> $memberAliases */
            $aliases += $memberAliases;

            foreach ([...$descriptor['libraries'], $descriptor['path']] as $library) {
                if ($library !== '') {
                    $libraries[$library] = $library;
                }
            }
        }

        $shared = NativeLibrary::composite(
            CdefComposer::merge($cdefs),
            $aliases,
            array_values($libraries),
        );
        self::$natives[$class] = $shared;
        self::$initialized[$class] = true;

        // Share the one scope with each member entity too, so a returned pointer
        // wrapped via a member's type class (which asks this registry for the
        // member's library) lives in the SAME scope the composite calls through.
        foreach ($origins as $origin) {
            if ($origin === $class) {
                continue;
            }
            self::$natives[$origin] = $shared;
            self::$initialized[$origin] = true;
            ArgumentMarshaller::rememberScalarsAllowed($origin, $scalarsAllowed);
        }
    }

    /**
     * The entity classes whose libraries a class is built from: the origin of each
     * `#[NativeLibraryComponent]`-tagged trait it `use`s, first-seen order.
     *
     * @param class-string $class
     * @return list<class-string>
     */
    private static function componentOrigins(string $class): array
    {
        $origins = [];
        foreach (class_uses($class) ?: [] as $trait) {
            foreach ((new \ReflectionClass($trait))->getAttributes(NativeLibraryComponent::class) as $attribute) {
                $origin = $attribute->newInstance()->origin;
                $origins[$origin] = $origin;
            }
        }

        return array_values($origins);
    }

    /**
     * Resolve an origin entity's generated library files and native metadata.
     *
     * @param class-string $origin
     * @return array{dir: string, cdef: string, aliases: string, path: string, hash: string, libraries: list<string>}
     */
    private static function libraryMetadata(string $origin): array
    {
        $info = self::info($origin);

        // The cdef/alias map sit beside the origin entity; walk up from its file in
        // case the entity is reached from a variant subdir.
        $directory = dirname((string) (new \ReflectionClass($origin))->getFileName());
        while ($info->cdef !== '' && !is_file($directory . '/' . $info->cdef) && dirname($directory) !== $directory) {
            $directory = dirname($directory);
        }

        return [
            'dir' => $directory,
            'cdef' => $directory . '/' . $info->cdef,
            'aliases' => $directory . '/' . $info->aliases,
            'path' => $info->path,
            'hash' => $info->hash,
            'libraries' => $info->libraries,
        ];
    }
}
