<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;
use Pnlx\Exception\ExtensionLoadException;

/**
 * Allocates standalone FFI C data outside of any loaded native library.
 *
 * Backed by a library-less `FFI::cdef('')` handle, it lets callers create raw
 * C buffers (e.g. for passing arrays of pointers into a native bridge) without
 * binding to a specific `.dylib`/`.so`. Exposed lazily via {@see \Pnlx\Runtime::allocator()}.
 */
class Allocator
{
    private FFI $ffi;

    public function __construct()
    {
        // Empty cdef yields an FFI scope with no symbols, usable only for allocation.
        $this->ffi = FFI::cdef('');
    }

    /**
     * Allocate a new C value of the given type declaration (e.g. `int`, `char[16]`).
     *
     * @throws ExtensionLoadException When FFI cannot allocate the requested type.
     */
    public function new(string $type): CData
    {
        // FFI::new() returns null when allocation fails (e.g. an unknown type).
        $value = $this->ffi->new($type);
        if ($value === null) {
            throw new ExtensionLoadException(sprintf('Failed to allocate C value of type %s.', $type));
        }

        return $value;
    }

    /**
     * Allocate a fixed-length C array of `void *` pointers.
     *
     * @throws ExtensionLoadException When the requested length is not positive.
     */
    public function voidPointerArray(int $length): CData
    {
        if ($length < 1) {
            throw new ExtensionLoadException('Pointer array length must be greater than zero.');
        }

        return $this->new(sprintf('void *[%d]', $length));
    }
}
