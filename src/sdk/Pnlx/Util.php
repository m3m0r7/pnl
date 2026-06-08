<?php

declare(strict_types=1);

namespace Pnlx;

use FFI;
use FFI\CData;
use Throwable;

class Util
{
    public function cString(mixed $value): string
    {
        return self::toString($value);
    }

    public static function toString(mixed $value): string
    {
        return is_string($value) ? $value : FFI::string($value);
    }

    public static function isNull(mixed $value): bool
    {
        if ($value === null) {
            return true;
        }

        if (!$value instanceof CData) {
            return false;
        }

        try {
            return FFI::isNull($value);
        } catch (Throwable) {
            return false;
        }
    }
}
