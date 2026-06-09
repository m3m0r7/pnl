<?php

declare(strict_types=1);

namespace Pnlx;

use FFI;
use FFI\CData;

/**
 * Static helpers for working with FFI values returned by native bridges.
 *
 * Used by generated extension code to convert C string pointers to PHP strings.
 * The null-pointer check lives as a function instead — see
 * {@see \Pnlx\Util\is_null()} in Util/functions.php.
 */
class Util
{
    /**
     * Convert a value to a PHP string, dereferencing FFI C string pointers.
     *
     * @param mixed $value Either a PHP string (returned as-is) or an {@see CData} pointer.
     * @throws \InvalidArgumentException When the value is neither a string nor a CData pointer.
     */
    public static function cString(mixed $value): string
    {
        if (is_string($value)) {
            return $value;
        }

        if (!$value instanceof CData) {
            throw new \InvalidArgumentException('Value must be a string or FFI\CData pointer.');
        }

        return FFI::string($value);
    }
}
