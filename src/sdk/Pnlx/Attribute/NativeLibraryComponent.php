<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/**
 * Marks a generated `<Class>LibraryComponent` trait — the method group of one
 * native library — and records the entity class that owns its
 * {@see NativeLibrary} metadata (cdef, library path, co-load libraries).
 *
 * {@see \Pnlx\Extension\AbstractExtension} reflects over a class's used traits at
 * boot: a single Component binds that one library, while several Components (a
 * composed class) are merged into one shared FFI scope, so a value from one
 * member's call can be passed to another's. Applied to a trait (which reflects as
 * `TARGET_CLASS`).
 */
#[Attribute(Attribute::TARGET_CLASS)]
final class NativeLibraryComponent
{
    /**
     * @param class-string $origin The entity class carrying this component's
     *                             {@see NativeLibrary} metadata.
     */
    public function __construct(public readonly string $origin)
    {
    }
}
