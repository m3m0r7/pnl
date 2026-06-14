<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI\CData;
use Pnlx\Exception\PHPNativeLibraryException;
use Pnlx\Helpers\CStaticTypeInterface;

/**
 * Converts generated-method arguments into the values handed to the native
 * bridge. Kept off the entity (called as `ArgumentMarshaller::scalarArg(...)`)
 * so a C function named `unwrap`/`scalarArg` can't shadow it.
 */
final class ArgumentMarshaller
{
    /**
     * Unwrap a pointer argument: a generated wrapper yields its inner value;
     * a raw \FFI\CData or null passes through untouched.
     */
    public static function unwrap(mixed $value): mixed
    {
        return $value instanceof CStaticTypeInterface ? $value->toValue() : $value;
    }

    /**
     * Coerce a scalar argument: a generated wrapper yields its inner value; a raw
     * PHP scalar is allowed only when `$allowRawScalars`
     * (`features.use_php_scalars_in_params`) is set, otherwise it must be wrapped.
     */
    public static function scalarArg(mixed $value, bool $allowRawScalars): mixed
    {
        if ($value instanceof CStaticTypeInterface) {
            return $value->toValue();
        }

        if ($value === null || $value instanceof CData) {
            return $value;
        }

        if (!$allowRawScalars) {
            throw new PHPNativeLibraryException(
                'Passing a raw PHP scalar requires features.use_php_scalars_in_params; '
                . 'wrap it instead (e.g. new \Pnlx\Helpers\Int_($value)).'
            );
        }

        return $value;
    }
}
