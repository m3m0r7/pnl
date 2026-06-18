<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/**
 * Marks a by-reference parameter that a C function takes as a pointer it writes
 * through — a scalar out/in-out (`int *`, `double *`), a string out (`char **`),
 * or a handle out (`T **`). The caller passes a plain variable; the dispatcher
 * allocates the matching C holder, hands it to the native call, and writes the
 * result back into the referenced variable.
 *
 * - `element` is the C type the holder is allocated of (`int`, `double`, or
 *   `void *` for the double-pointer cases).
 * - `string` writes the result back as a PHP string (`FFI::string`), for `char **`.
 * - `wrap` writes the result back wrapped in the named class (a `T **` handle);
 *   `null` with `string === false` writes a bare scalar/CData.
 * - `buffer` marks a writable single-level `char *` byte buffer: the caller passes
 *   a pre-sized string (its length is the capacity), the dispatcher copies it into a
 *   `char[len]`, hands that to the call, and writes all `len` bytes back (binary
 *   safe, so callers `substr`/`rtrim` as needed).
 */
#[Attribute(Attribute::TARGET_PARAMETER)]
final class NativePointer
{
    public function __construct(
        public readonly string $element,
        public readonly bool $string = false,
        public readonly ?string $wrap = null,
        public readonly bool $buffer = false,
    ) {
    }
}
