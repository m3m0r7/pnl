<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/**
 * Install-time metadata for a generated extension entity, attached as a class
 * attribute instead of as `const`s so a composed class (which mixes in several
 * library Components) carries each member's data without colliding constants.
 *
 * {@see \Pnlx\Extension\AbstractExtension} reads this via reflection when booting
 * the native library. `path`/`hash` describe the resolved native library and are
 * stamped in at install time; `cdef`/`aliases` are the sibling generated files;
 * `libraries` are extra co-load library paths; `bootToken` is the hard-to-collide
 * one-time-boot sentinel.
 */
#[Attribute(Attribute::TARGET_CLASS)]
final class NativeLibrary
{
    /**
     * @param list<string> $libraries Absolute paths of extra shared libraries to
     *                                co-load alongside this one.
     */
    public function __construct(
        public readonly string $name,
        public readonly string $version,
        public readonly string $description = '',
        public readonly string $path = '',
        public readonly string $hash = '',
        public readonly string $cdef = '',
        public readonly string $aliases = 'function.aliases.php',
        public readonly array $libraries = [],
        public readonly string $bootToken = '',
    ) {
    }
}
