<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

abstract class AbstractInteger implements AnySizeInteger
{
    protected const UNSIGNED = false;

    /** The value as a 64-bit two's-complement pattern held in a PHP int. */
    private readonly int $value;

    public function __construct(int|string|self $value)
    {
        $this->value = self::fold($value);
    }

    /**
     * Reduce any accepted input to the 64-bit two's-complement pattern. A decimal
     * string above PHP_INT_MAX (an unsigned value) is folded into 64 bits using
     * two 32-bit halves, so no arbitrary-precision extension (bcmath/gmp) is used.
     */
    private static function fold(int|string|self $value): int
    {
        if ($value instanceof self) {
            return $value->value;
        }

        // `\is_int` (not this namespace's wrapper-aware Helpers\is_int): narrow the
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

    public function __toString(): string
    {
        if (static::UNSIGNED && $this->value < 0) {
            // Render the unsigned view of a value whose top bit is set.
            return sprintf('%u', $this->value);
        }

        return (string) $this->value;
    }

    public function toInt(): int
    {
        return $this->value;
    }

    public function toValue(): int
    {
        // PHP FFI marshals a scalar argument from a PHP scalar (it cannot take a
        // scalar \FFI\CData) and keeps the integer's low bits, so the stored
        // two's-complement pattern passes losslessly — even an unsigned value
        // above PHP_INT_MAX, handed over as its negative signed view.
        return $this->value;
    }
}
