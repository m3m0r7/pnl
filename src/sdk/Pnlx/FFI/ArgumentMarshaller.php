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
     * Per-class `features.use_php_scalars_in_params`, resolved against the right
     * project root when the extension boots (see {@see rememberScalarsAllowed()}).
     *
     * @var array<class-string, bool>
     */
    private static array $scalarsAllowed = [];

    /**
     * Record whether a class may take raw PHP scalars. Called from the extension's
     * one-time boot, where the project root is resolved correctly — unlike a lazy
     * lookup at call time, which has no workspace context.
     *
     * @param class-string $class
     */
    public static function rememberScalarsAllowed(string $class, bool $allowed): void
    {
        self::$scalarsAllowed[$class] = $allowed;
    }

    /**
     * Unwrap a pointer argument: a generated wrapper yields its inner value;
     * a raw \FFI\CData or null passes through untouched.
     */
    public static function unwrap(mixed $value): mixed
    {
        return $value instanceof CStaticTypeInterface ? $value->toValue() : $value;
    }

    /**
     * Coerce a scalar argument for a call on `$class`: a generated wrapper yields
     * its inner value; a raw PHP scalar is allowed only when that class's
     * `features.use_php_scalars_in_params` is set, otherwise it must be wrapped.
     *
     * @param class-string $class
     */
    public static function scalarArg(string $class, mixed $value): mixed
    {
        if ($value instanceof CStaticTypeInterface) {
            return $value->toValue();
        }

        if ($value === null || $value instanceof CData) {
            return $value;
        }

        if (!(self::$scalarsAllowed[$class] ?? false)) {
            throw new PHPNativeLibraryException(
                'Passing a raw PHP scalar requires features.use_php_scalars_in_params; '
                . 'wrap it instead (e.g. new \Pnlx\Helpers\Int_($value)).'
            );
        }

        return $value;
    }
}
