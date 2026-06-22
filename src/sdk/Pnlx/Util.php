<?php

declare(strict_types=1);

namespace Pnlx;

use FFI;
use FFI\CData;

/**
 * Static helpers for working with FFI values returned by native libraries.
 *
 * Used by generated extension code to convert C string pointers to PHP strings
 * and to null-check returned pointers. A richer null check that also shadows
 * PHP's built-in is also available as a function — see {@see \Pnlx\Util\is_null()}
 * in Util/functions.php — but generated code calls {@see isNull()} here so it
 * works without that function file being required.
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

    /** Upper bound on a wide string's length, so a missing NUL cannot loop forever. */
    private const int WIDE_STRING_MAX = 65536;

    /**
     * Decode a NUL-terminated wide string (`wchar_t *`) to a PHP UTF-8 string, or
     * null for a NULL pointer. On macOS/Linux `wchar_t` is 4 bytes (UTF-32), which
     * the cdef types as `int`, so the pointer indexes as 32-bit code points; each is
     * turned into UTF-8 with {@see \mb_chr()}. Used by generated accessors for
     * wide-string struct fields (e.g. hidapi's `manufacturer_string`).
     *
     * @param mixed $value A {@see CData} `wchar_t *` (possibly NULL), or null.
     */
    public static function wcStringOrNull(mixed $value): ?string
    {
        if ($value === null || !$value instanceof CData) {
            return null;
        }

        try {
            if (FFI::isNull($value)) {
                return null;
            }
        } catch (\Throwable) {
            return null;
        }

        $out = '';
        for ($i = 0; $i < self::WIDE_STRING_MAX; $i++) {
            // Each element is a 32-bit code point; a non-int or the NUL terminates.
            $codepoint = $value[$i];
            if (!is_int($codepoint) || $codepoint === 0) {
                return $out;
            }
            // Skip anything outside the Unicode range (a stray/garbage value).
            if ($codepoint > 0 && $codepoint <= 0x10FFFF) {
                $out .= \mb_chr($codepoint, 'UTF-8');
            }
        }

        return $out;
    }

    /**
     * Null-pointer check used by generated struct field accessors. Mirrors
     * {@see \Pnlx\Util\is_null()} but as a static method so generated code does
     * not depend on the Util/functions.php file being loaded.
     *
     * A generated {@see \Pnlx\Types\PointerInterface} wrapper reports the pointer
     * it wraps; a {@see \Pnlx\Types\Null_} value object is always null; a raw FFI
     * {@see CData} pointer is checked via {@see FFI::isNull()} (any failure means
     * "not null", since that method only accepts pointer CData); anything else
     * falls back to PHP's native `\is_null`.
     */
    public static function isNull(mixed $value): bool
    {
        if ($value instanceof \Pnlx\Types\Null_) {
            return true;
        }

        if ($value instanceof \Pnlx\Types\PointerInterface) {
            $value = $value->cdata();
        }

        if (!$value instanceof CData) {
            return \is_null($value);
        }

        try {
            return FFI::isNull($value);
        } catch (\Throwable) {
            return false;
        }
    }
}
