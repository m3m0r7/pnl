<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;

/**
 * Allocates C values in a specific extension's FFI scope.
 *
 * Allocating in the *extension's* scope (not a throwaway `FFI::cdef('')`) is what
 * makes the value type-compatible with that extension's functions — a struct made
 * in a foreign scope cannot be passed to them. Values are owned by PHP's GC: the
 * memory lives exactly as long as the returned {@see CData} is referenced, so keep
 * the reference alive while the native side still holds a pointer into it (use an
 * {@see AllocationScope} when several allocations must outlive the call together).
 */
final class Allocator
{
    public function __construct(private readonly NativeLibrary $library)
    {
    }

    /**
     * Allocate a zero-initialized value of a C type declared in this extension's
     * cdef (e.g. `struct point`, `int`, `double[4]`). GC-owned.
     */
    public function new(string $type): CData
    {
        return $this->library->allocate($type);
    }

    /**
     * Allocate a NUL-terminated `char` buffer holding `$value`'s bytes, for an API
     * that takes a writable `char *`. GC-owned.
     */
    public function cString(string $value): CData
    {
        $length = \strlen($value);
        // `allocate` zero-initializes, so the trailing byte is already the NUL.
        $buffer = $this->library->allocate(\sprintf('char[%d]', $length + 1));
        if ($length > 0) {
            FFI::memcpy($buffer, $value, $length);
        }

        return $buffer;
    }

    /**
     * Open a scope that retains every allocation made through it until it is
     * released, so their pointers can be handed to C without a stray garbage
     * collection freeing the backing memory mid-call.
     */
    public function scope(): AllocationScope
    {
        return new AllocationScope($this);
    }
}
