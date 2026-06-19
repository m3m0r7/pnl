<?php

declare(strict_types=1);

namespace Pnlx;

use FFI;
use FFI\CData;

/**
 * Static helpers for working with FFI values returned by native libraries.
 *
 * Used by generated extension code to convert C string pointers to PHP strings.
 * The null-pointer check lives as a function instead — see
 * {@see \Pnlx\Util\is_null()} in Util/functions.php.
 */
class Util
{
    /** A symbol-less FFI scope used only to cast a byte pointer to `char *`. */
    private static ?FFI $scope = null;

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

        // `FFI::string()` only accepts a `char *`; a different byte pointer (a
        // `uint8_t *`/`unsigned char *` such as GLEW's `GLubyte *`) is cast first.
        $charPointer = (self::$scope ??= FFI::cdef(''))->cast('char *', $value);

        return $charPointer instanceof CData ? FFI::string($charPointer) : '';
    }

    /**
     * Like {@see cString()}, but a NULL `char *` becomes PHP null instead of
     * dereferencing a null pointer. Many C functions return NULL to mean "absent"
     * (getenv() of an unset name, strstr()/strchr() with no match,
     * idn2_check_version() below the requested version), so every generated
     * `char *` return is routed through this and the value can be nullable.
     *
     * @param mixed $value A PHP string, an {@see CData} pointer (possibly NULL), or null.
     */
    public static function cStringOrNull(mixed $value): ?string
    {
        if ($value === null) {
            return null;
        }

        if ($value instanceof CData) {
            try {
                // FFI::isNull() only accepts a pointer CData; a char * always is one.
                if (FFI::isNull($value)) {
                    return null;
                }
            } catch (\Throwable) {
                // Not a pointer — let cString() handle (or reject) it below.
            }
        }

        return self::cString($value);
    }
}
