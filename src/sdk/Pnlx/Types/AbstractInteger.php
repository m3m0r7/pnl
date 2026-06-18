<?php

declare(strict_types=1);

namespace Pnlx\Types;

abstract class AbstractInteger implements AnySizeInteger, PointerInterface
{
    protected const UNSIGNED = false;

    /** The fundamental C type this wraps, used when allocating storage. */
    protected const string C_TYPE = 'int';

    /** A symbol-less FFI scope shared by every integer wrapper, for allocation only. */
    private static ?\FFI $scope = null;

    /** The value as a 64-bit two's-complement pattern; null when storage is allocated. */
    private readonly ?int $value;

    /** Allocated `C_TYPE[..]` storage; null for a plain value. */
    private readonly ?\FFI\CData $cdata;

    /**
     * The argument decides the shape:
     *   - `new Int_(5)` — a value handed straight to the call.
     *   - `new Int_()` / `new Int_(null)` — a single cell (C `int i;`, zero-filled).
     *   - `new Int_([1, 2, 3])` — `int[3] = {1, 2, 3}`.
     *   - `new Int_([[1, 2], [3, 4]])` — `int[2][2]` (a nested array becomes a
     *     contiguous multidimensional array; every sub-array must share a length).
     * Array elements that are `null` become 0. For a fixed-size buffer whose contents
     * don't matter (an out-array), use {@see alloc()}.
     *
     * @param int|string|array<mixed>|null $value
     */
    final public function __construct(int|string|array|null $value = null)
    {
        if ($value === []) {
            // An empty initialiser is "no elements" — a NULL pointer (FFI cannot
            // allocate a zero-length array, and C APIs pass NULL with a count of 0).
            $this->value = null;
            $this->cdata = null;

            return;
        }

        if (is_array($value)) {
            $dims = self::shape($value);
            self::ensureRectangular($value, $dims);
            $type = static::C_TYPE . implode('', array_map(static fn (int $d): string => "[$d]", $dims));
            $cdata = self::allocate($type);
            self::fill($cdata, $value);
            $this->cdata = $cdata;
            $this->value = null;

            return;
        }

        if ($value === null) {
            $this->cdata = self::allocate(static::C_TYPE . '[1]');
            $this->value = null;

            return;
        }

        $this->value = self::fold($value);
        $this->cdata = null;
    }

    /**
     * Allocate a fixed-size `C_TYPE[$size]` buffer whose contents start zero-filled —
     * for an out-array the native call writes into (e.g. `UnsignedChar::alloc(16)`).
     */
    public static function alloc(int $size): static
    {
        return new static(array_fill(0, max(1, $size), null));
    }

    /** The fundamental C type this wraps (e.g. `int`, `unsigned char`). */
    public static function cType(): string
    {
        return static::C_TYPE;
    }

    /**
     * The dimensions of a (possibly nested) initialiser, taken from the first element
     * at each level: `[1,2,3]` → `[3]`, `[[1,2],[3,4]]` → `[2,2]`.
     *
     * @param array<mixed> $value
     *
     * @return list<int>
     */
    private static function shape(array $value): array
    {
        $dims = [];
        $cursor = $value;
        while (is_array($cursor)) {
            $count = count($cursor);
            if ($count === 0) {
                throw new \Pnlx\Exception\PHPNativeLibraryException(
                    'Cannot allocate a zero-length array.'
                );
            }
            $dims[] = $count;
            $cursor = array_values($cursor)[0];
        }

        return $dims;
    }

    /**
     * Reject a ragged initialiser: every node at depth `d` must be an array of length
     * `$dims[$d]` until the leaves, so the array maps onto a contiguous `T[n][m]…`.
     *
     * @param list<int> $dims
     */
    private static function ensureRectangular(mixed $value, array $dims): void
    {
        if ($dims === []) {
            if (is_array($value)) {
                throw new \Pnlx\Exception\PHPNativeLibraryException(
                    'Nested arrays must all have the same depth (got a ragged array).'
                );
            }

            return;
        }

        if (!is_array($value) || count($value) !== $dims[0]) {
            throw new \Pnlx\Exception\PHPNativeLibraryException(
                'Nested arrays must all share a length (got a ragged array).'
            );
        }

        $rest = array_slice($dims, 1);
        foreach ($value as $element) {
            self::ensureRectangular($element, $rest);
        }
    }

    /**
     * Write a (possibly nested) initialiser into allocated storage, recursing into
     * sub-arrays for each further dimension. `null` leaves become 0.
     *
     * @param array<mixed> $values
     */
    private static function fill(\FFI\CData $cell, array $values): void
    {
        $index = 0;
        foreach (array_values($values) as $value) {
            if (is_array($value)) {
                $sub = self::elementAt($cell, $index);
                if ($sub instanceof \FFI\CData) {
                    self::fill($sub, $value);
                }
            } elseif ($value === null) {
                self::writeElement($cell, $index, 0);
            } elseif (is_int($value) || is_string($value)) {
                self::writeElement($cell, $index, self::fold($value));
            } else {
                self::writeElement($cell, $index, 0);
            }
            $index++;
        }
    }

    /**
     * Reduce any accepted input to the 64-bit two's-complement pattern. A decimal
     * string above PHP_INT_MAX (an unsigned value) is folded into 64 bits using
     * two 32-bit halves, so no arbitrary-precision extension (bcmath/gmp) is used.
     */
    private static function fold(int|string|self $value): int
    {
        if ($value instanceof self) {
            return $value->toInt();
        }

        // `\is_int` (not this namespace's wrapper-aware Types\is_int): narrow the
        // remaining union to the decimal-string case below.
        if (\is_int($value)) {
            return $value;
        }

        $negative = str_starts_with($value, '-');
        $digits = $negative ? substr($value, 1) : $value;
        $low = 0;
        $high = 0;
        $length = strlen($digits);
        for ($i = 0; $i < $length; $i++) {
            $digit = ord($digits[$i]) - 48; // '0'
            if ($digit < 0 || $digit > 9) {
                continue;
            }

            $low = $low * 10 + $digit;
            $high = $high * 10 + intdiv($low, 0x100000000);
            $low &= 0xFFFFFFFF;
            $high &= 0xFFFFFFFF;
        }

        $folded = ($high << 32) | $low;

        return $negative ? -$folded : $folded;
    }

    /** Allocate storage of the given C type in the shared library-less FFI scope. */
    private static function allocate(string $type): \FFI\CData
    {
        $value = (self::$scope ??= \FFI::cdef(''))->new($type);
        if (!$value instanceof \FFI\CData) {
            throw new \Pnlx\Exception\PHPNativeLibraryException(
                sprintf('Failed to allocate C value of type %s.', $type)
            );
        }

        return $value;
    }

    /**
     * Write one element into a C array holder. Routed through a typed-`CData`
     * parameter so the holder keeps its `FFI\CData` type — a direct
     * `$holder[$i] = ...` on a local widens it to `mixed` under static analysis.
     * Public so static analysis treats the offset write as an intended side effect.
     */
    public static function writeElement(\FFI\CData $holder, int $index, int $value): void
    {
        $holder[$index] = $value;
    }

    /** Read one element (a deeper dimension's sub-array, or a leaf) from a holder. */
    private static function elementAt(\FFI\CData $holder, int $index): mixed
    {
        return $holder[$index];
    }

    public function __toString(): string
    {
        $value = $this->toInt();
        if (static::UNSIGNED && $value < 0) {
            // Render the unsigned view of a value whose top bit is set.
            return sprintf('%u', $value);
        }

        return (string) $value;
    }

    public function toInt(): int
    {
        if ($this->cdata !== null) {
            // Allocated storage: read back its first cell (what the native call wrote).
            $cell = self::elementAt($this->cdata, 0);

            return is_numeric($cell) ? (int) $cell : 0;
        }

        return $this->value ?? 0;
    }

    public function toValue(): mixed
    {
        // PHP FFI marshals a scalar argument from a PHP scalar (it cannot take a
        // scalar \FFI\CData) and keeps the integer's low bits, so the stored
        // two's-complement pattern passes losslessly — even an unsigned value
        // above PHP_INT_MAX, handed over as its negative signed view. Allocated
        // storage instead hands over its CData, which decays to a pointer.
        return $this->cdata ?? $this->value;
    }

    public function cdata(): \FFI\CData
    {
        return $this->cdata ?? throw new \LogicException(sprintf(
            '%s holds a plain value, not allocated storage; construct it from null '
            . 'or an array (or via %s::alloc()) to get a pointer.',
            static::class,
            static::class,
        ));
    }
}
