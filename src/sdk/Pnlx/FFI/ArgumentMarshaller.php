<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI\CData;
use Pnlx\Exception\PHPNativeLibraryException;
use Pnlx\Types\ValueInterface;

/**
 * Converts generated-method arguments into the values handed to the native
 * library. Kept off the entity (called as `ArgumentMarshaller::scalarArg(...)`)
 * so a C function named `unwrap`/`scalarArg` can't shadow it.
 */
final class ArgumentMarshaller
{
    /**
     * Per-class `features.scalar_params`, resolved against the right
     * project root when the extension boots (see {@see rememberScalarsAllowed()}).
     *
     * @var array<class-string, bool>
     */
    private static array $scalarsAllowed = [];

    /**
     * Resolved exported-data-symbol `\FFI\CData`s, keyed by marker class. A symbol is
     * resolved once and then held for the request: C may keep the pointer after the
     * call returns (e.g. an encoding handed to a regex), so the CData must not be
     * garbage-collected. This is the internal replacement for the old GlobalMemory
     * store — callers never see it; they just pass the marker class-string.
     *
     * @var array<class-string, mixed>
     */
    private static array $symbols = [];

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
     * Unwrap a pointer argument: a generated wrapper yields its inner value; an
     * exported-data-symbol marker class-string (e.g. `\Pnlx\Liboniguruma\OnigEncodingUTF8::class`)
     * is resolved to its `\FFI\CData` and held for the request; a raw \FFI\CData,
     * null, or plain string (a `void *`/`char *` buffer) passes through untouched.
     */
    public static function unwrap(mixed $value): mixed
    {
        if ($value instanceof ValueInterface) {
            return $value->toValue();
        }

        if (is_string($value) && is_a($value, SymbolInterface::class, true)) {
            return self::$symbols[$value] ??= self::resolveSymbol($value);
        }

        return $value;
    }

    /**
     * Marshal a struct passed BY VALUE (e.g. quickjs's `JSValue`, gdbm's `datum`).
     * Its wrapper normalises to a `T *` pointer (so field accessors and pointer uses
     * work), but the native function wants the struct itself — FFI passes a struct
     * argument by value. Dereference a pointer to the struct value; a value `\FFI\CData`
     * is forwarded as-is.
     */
    public static function structValue(mixed $value): mixed
    {
        $cdata = $value instanceof ValueInterface ? $value->toValue() : $value;

        if (!$cdata instanceof CData) {
            return $cdata;
        }

        $kind = \FFI::typeof($cdata)->getKind();

        return $kind === \FFI\CType::TYPE_POINTER || $kind === \FFI\CType::TYPE_ARRAY
            ? $cdata[0]
            : $cdata;
    }

    /**
     * Resolve an exported-data-symbol marker to its `\FFI\CData` in the owning
     * extension's FFI scope: a pointer symbol yields its value, a data symbol yields
     * its address.
     *
     * @param class-string<SymbolInterface> $symbolClass
     */
    private static function resolveSymbol(string $symbolClass): mixed
    {
        $library = NativeLibraryRegistry::of($symbolClass::extension());

        return match ($symbolClass::mode()) {
            SymbolMode::Value => $library->global($symbolClass::name()),
            SymbolMode::Address => $library->addressOf($symbolClass::name()),
        };
    }

    /**
     * Coerce a scalar argument for a call on `$class`: a generated wrapper yields
     * its inner value; a raw PHP scalar is allowed only when that class's
     * `features.scalar_params` is set, otherwise it must be wrapped.
     *
     * @param class-string $class
     */
    public static function scalarArg(string $class, mixed $value): mixed
    {
        if ($value instanceof ValueInterface) {
            return $value->toValue();
        }

        // A generated PHP enum (int-backed) sent where the C function takes the
        // enum: hand the native call its backing integer value.
        if ($value instanceof \BackedEnum) {
            return $value->value;
        }

        if ($value === null || $value instanceof CData) {
            return $value;
        }

        if (!(self::$scalarsAllowed[$class] ?? false)) {
            throw new PHPNativeLibraryException(
                'Passing a raw PHP scalar requires features.scalar_params; '
                . 'wrap it instead (e.g. new \Pnlx\Types\Int_($value)).'
            );
        }

        return $value;
    }
}
