<?php

declare(strict_types=1);

namespace Pnlx\Types;

abstract class AbstractFloat implements AnyFloat, PointerInterface
{
    /** The fundamental C type this wraps, used when allocating storage. */
    protected const string C_TYPE = 'double';

    /** A symbol-less FFI scope shared by every float wrapper, for allocation only. */
    private static ?\FFI $scope = null;

    /** The value; null when storage is allocated. */
    private readonly ?float $value;

    /** Allocated `C_TYPE[..]` storage; null for a plain value. */
    private readonly ?\FFI\CData $cdata;

    /**
     * The argument decides the shape:
     *   - `new Double(3.5)` — a value handed straight to the call.
     *   - `new Double()` / `new Double(null)` — a single cell (zero-filled).
     *   - `new Double([1.0, 2.0])` — `double[2] = {1.0, 2.0}`.
     *   - `new Double([[1, 2], [3, 4]])` — `double[2][2]` (a nested array becomes a
     *     contiguous multidimensional array; every sub-array must share a length).
     * Array elements that are `null` become 0. For a fixed-size buffer whose contents
     * don't matter (an out-array), use {@see alloc()}.
     *
     * @param float|int|string|array<mixed>|null $value
     */
    final public function __construct(float|int|string|array|null $value = null)
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

        $this->value = (float) $value;
        $this->cdata = null;
    }

    /**
     * Allocate a fixed-size `C_TYPE[$size]` buffer whose contents start zero-filled —
     * for an out-array the native call writes into (e.g. `Double::alloc(4)`).
     */
    public static function alloc(int $size): static
    {
        return new static(array_fill(0, max(1, $size), null));
    }

    /** The fundamental C type this wraps (e.g. `double`, `float`). */
    public static function cType(): string
    {
        return static::C_TYPE;
    }

    /**
     * The dimensions of a (possibly nested) initialiser, taken from the first element
     * at each level: `[1,2]` → `[2]`, `[[1,2],[3,4]]` → `[2,2]`.
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
            } elseif (is_numeric($value)) {
                self::writeElement($cell, $index, (float) $value);
            } else {
                self::writeElement($cell, $index, 0.0);
            }
            $index++;
        }
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
    public static function writeElement(\FFI\CData $holder, int $index, float $value): void
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
        return (string) $this->toFloat();
    }

    public function toFloat(): float
    {
        if ($this->cdata !== null) {
            // Allocated storage: read back its first cell (what the native call wrote).
            $cell = self::elementAt($this->cdata, 0);

            return is_numeric($cell) ? (float) $cell : 0.0;
        }

        return $this->value ?? 0.0;
    }

    public function toValue(): mixed
    {
        // A plain value hands over the PHP float; allocated storage hands over its
        // CData, which decays to a pointer at the FFI boundary.
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
