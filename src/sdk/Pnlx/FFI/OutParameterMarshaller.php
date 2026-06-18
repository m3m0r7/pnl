<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;
use Pnlx\Attribute\NativePointer;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Types\ValueInterface;
use ReflectionMethod;

/**
 * Dispatches a generated method whose signature has one or more by-reference
 * {@see NativePointer} parameters (a C `int *`/`double *` out/in-out argument).
 *
 * For each such parameter the caller may pass:
 *   - a scalar (or null) — a single-element out/in-out value, or
 *   - a PHP array — a buffer of `count()` elements.
 * The dispatcher allocates the matching C holder in its own library-less FFI
 * scope (out-parameter holders are always fundamental — `int[1]`, `double[2]`,
 * `void *[1]`), copies any input in, hands the holder to the native call, then
 * writes the value(s) the call produced back into the referenced variable.
 * Arguments without the attribute are passed straight through (the generated
 * method has already marshalled them).
 */
final class OutParameterMarshaller
{
    /**
     * A symbol-less `FFI::cdef('')` scope, used only to allocate the fundamental C
     * holders for out-parameters. Lazily created and reused for the request.
     */
    private static ?FFI $scope = null;

    /**
     * @param class-string         $class      The generated entity class.
     * @param string               $phpMethod  The PHP method name (for reflection).
     * @param string               $symbol     The native C symbol to dispatch.
     * @param array<int, mixed>    $arguments  Positional args; NativePointer slots
     *                                         hold a reference to write back into.
     */
    public static function call(string $class, string $phpMethod, string $symbol, array $arguments): mixed
    {
        $parameters = (new ReflectionMethod($class, $phpMethod))->getParameters();
        $native = [];
        /** @var array<int, array{pointer: NativePointer, holder: CData, count: int|null, buffer: bool, size: int}> $writeBacks */
        $writeBacks = [];

        foreach ($parameters as $index => $parameter) {
            $attributes = $parameter->getAttributes(NativePointer::class);
            $value = $arguments[$index] ?? null;

            // No NativePointer, or the caller passed a ready C pointer/buffer:
            // forward the (already-marshalled) value untouched.
            if ($attributes === [] || $value instanceof CData) {
                $native[] = $value;
                continue;
            }

            $pointer = $attributes[0]->newInstance();
            $element = $pointer->element;

            // A writable `char *` byte buffer: null is a NULL pointer; otherwise the
            // caller's string is the pre-sized capacity. Copy it into a char[len],
            // hand the buffer over, and read all len bytes back (binary safe).
            if ($pointer->buffer) {
                if ($value === null) {
                    $native[] = null;
                    continue;
                }
                $bytes = is_string($value) ? $value : '';
                if ($value instanceof ValueInterface) {
                    $inner = $value->toValue();
                    $bytes = is_string($inner) ? $inner : '';
                }
                $size = strlen($bytes);
                $holder = self::allocate(sprintf('char[%d]', max(1, $size)));
                if ($size > 0) {
                    FFI::memcpy($holder, $bytes, $size);
                }
                $native[] = $holder;
                $writeBacks[$index] = ['pointer' => $pointer, 'holder' => $holder, 'count' => null, 'buffer' => true, 'size' => $size];
                continue;
            }

            if (is_array($value)) {
                $count = max(1, count($value));
                $holder = self::allocate(sprintf('%s[%d]', $element, $count));
                $slot = 0;
                foreach ($value as $item) {
                    self::writeElement($holder, $slot++, self::scalar($item));
                }
                $native[] = $holder;
                $writeBacks[$index] = ['pointer' => $pointer, 'holder' => $holder, 'count' => $count, 'buffer' => false, 'size' => 0];
            } else {
                $holder = self::allocate(sprintf('%s[1]', $element));
                if ($value !== null) {
                    self::writeElement($holder, 0, self::scalar($value));
                }
                $native[] = $holder;
                $writeBacks[$index] = ['pointer' => $pointer, 'holder' => $holder, 'count' => null, 'buffer' => false, 'size' => 0];
            }
        }

        $result = $class::__callStatic($symbol, $native);

        foreach ($writeBacks as $index => $writeBack) {
            $holder = $writeBack['holder'];
            if ($writeBack['buffer']) {
                // Read back the whole buffer (binary safe); the caller trims by the
                // length the call reported in a separate out-parameter.
                $arguments[$index] = $writeBack['size'] > 0 ? FFI::string($holder, $writeBack['size']) : '';
                continue;
            }
            if ($writeBack['count'] !== null) {
                $values = [];
                for ($slot = 0; $slot < $writeBack['count']; $slot++) {
                    $values[$slot] = self::writeBack($writeBack['pointer'], self::readElement($holder, $slot));
                }
                $arguments[$index] = $values;
                continue;
            }

            $arguments[$index] = self::writeBack($writeBack['pointer'], self::readElement($holder, 0));
        }

        return $result;
    }

    /**
     * Allocate a fundamental C holder (`int[1]`, `double[2]`, `void *[1]`) in this
     * marshaller's library-less FFI scope.
     *
     * @throws ExtensionLoadException When FFI cannot allocate the requested type.
     */
    private static function allocate(string $type): CData
    {
        // Empty cdef yields an FFI scope with no symbols, usable only for allocation.
        $value = (self::$scope ??= FFI::cdef(''))->new($type);
        if (!$value instanceof CData) {
            throw new ExtensionLoadException(sprintf('Failed to allocate C value of type %s.', $type));
        }

        return $value;
    }

    /**
     * Write one element into a C array holder. Routed through a typed-`CData`
     * parameter so the holder keeps its `FFI\CData` type — a direct
     * `$holder[$i] = ...` on a local widens it to `mixed` under static analysis.
     * Public so static analysis treats the offset write as an intended side effect.
     */
    public static function writeElement(CData $holder, int $index, mixed $value): void
    {
        $holder[$index] = $value;
    }

    /** Read one element from a C array holder (kept off the caller for the reason above). */
    public static function readElement(CData $holder, int $index): mixed
    {
        return $holder[$index];
    }

    /** Convert one written-back C value to the PHP form the parameter declares. */
    private static function writeBack(NativePointer $pointer, mixed $out): mixed
    {
        // A null pointer cell reads back as PHP null (e.g. a `char **`/`T **` the
        // call left unset); surface it as null rather than wrapping/stringifying.
        $isNull = $out === null || ($out instanceof CData && FFI::isNull($out));

        // A `char **`/`uint8_t **` result is a C string (or NULL). The holder is a
        // `void *`, which FFI::string() rejects, so cast it to `char *` first.
        if ($pointer->string) {
            return $isNull || !$out instanceof CData ? null : self::readCString($out);
        }

        // A `T **` handle result is wrapped in the package's pointer/context class.
        if ($pointer->wrap !== null) {
            if ($isNull) {
                return null;
            }
            $class = $pointer->wrap;

            return new $class($out);
        }

        // A scalar out parameter: the value is already a PHP scalar.
        return $out;
    }

    /**
     * Read a NUL-terminated C string from a `void *` pointer written back by a
     * native call (a `char **`/`uint8_t **` out-parameter holder). The pointer is
     * cast to `char *` first — `FFI::string()` rejects a bare `void *`.
     */
    private static function readCString(CData $pointer): string
    {
        $charPointer = (self::$scope ??= FFI::cdef(''))->cast('char *', $pointer);
        if (!$charPointer instanceof CData) {
            throw new ExtensionLoadException('Failed to cast pointer to char *.');
        }

        return FFI::string($charPointer);
    }

    /** Unwrap a helper value (`Int_`, `AnyFloat`, …) to the scalar FFI accepts. */
    private static function scalar(mixed $value): mixed
    {
        return $value instanceof ValueInterface ? $value->toValue() : $value;
    }
}
