<?php

declare(strict_types=1);

namespace Pnlx\Util;

use FFI;
use FFI\CData;
use Throwable;

/**
 * Null check that understands FFI pointers as well as ordinary PHP values.
 *
 * Import it to shadow PHP's built-in `is_null` in a file that mixes native
 * return values with regular values:
 *
 *     use function Pnlx\Util\is_null;
 *
 * For an FFI {@see CData} pointer it reports whether the pointer is null (via
 * {@see FFI::isNull()}, treating any failure as "not null" since that method
 * only accepts pointer CData). For anything else it falls back to PHP's native
 * `\is_null`, so it is a safe drop-in replacement in normal code too.
 */
function is_null(mixed $value): bool
{
    if (!$value instanceof CData) {
        return \is_null($value);
    }

    try {
        return FFI::isNull($value);
    } catch (Throwable) {
        return false;
    }
}
