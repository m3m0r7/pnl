<?php

declare(strict_types=1);

namespace Pnlx\Util;

use FFI;
use FFI\CData;
use Pnlx\Helpers\AnyFloat;
use Pnlx\Helpers\AnySizeInteger;
use Pnlx\Helpers\ContextInterface;
use Pnlx\Helpers\Null_;
use Pnlx\Helpers\String_;
use Throwable;

/**
 * Null check that understands generated pointer wrappers and FFI pointers as
 * well as ordinary PHP values.
 *
 * Import it to shadow PHP's built-in `is_null` in a file that mixes native
 * return values with regular values:
 *
 *     use function Pnlx\Util\is_null;
 *
 * A generated {@see ContextInterface} pointer wrapper reports whether the pointer
 * it wraps is null, so `is_null($renderer)` works directly without reaching for
 * `$renderer->cdata()`. A {@see Null_} value object is always null. A raw FFI
 * {@see CData} pointer is checked via {@see FFI::isNull()} (any failure means
 * "not null", since that method only accepts pointer CData). Anything else falls
 * back to PHP's native `\is_null`, so it is a safe drop-in replacement.
 */
function is_null(mixed $value): bool
{
    if ($value instanceof Null_) {
        return true;
    }

    if ($value instanceof ContextInterface) {
        $value = $value->cdata();
    }

    if (!$value instanceof CData) {
        return \is_null($value);
    }

    try {
        return FFI::isNull($value);
    } catch (Throwable) {
        return false;
    }
}

/** Whether $value is a PHP int or a wrapped C integer. */
function is_int(mixed $value): bool
{
    return \is_int($value) || $value instanceof AnySizeInteger;
}

/** Alias of {@see is_int()}. */
function is_integer(mixed $value): bool
{
    return is_int($value);
}

/** Alias of {@see is_int()} (PHP has no distinct long type). */
function is_long(mixed $value): bool
{
    return is_int($value);
}

/** Whether $value is a PHP float or a wrapped C floating-point value. */
function is_float(mixed $value): bool
{
    return \is_float($value) || $value instanceof AnyFloat;
}

/** Alias of {@see is_float()}. */
function is_double(mixed $value): bool
{
    return is_float($value);
}

/** Whether $value is a PHP string or a wrapped C string. */
function is_string(mixed $value): bool
{
    return \is_string($value) || $value instanceof String_;
}

/** Like PHP's gettype(), but reports wrapped values as their scalar type. */
function gettype(mixed $value): string
{
    if ($value instanceof AnySizeInteger) {
        return 'integer';
    }
    if ($value instanceof AnyFloat) {
        return 'double';
    }
    if ($value instanceof String_) {
        return 'string';
    }

    return \gettype($value);
}
