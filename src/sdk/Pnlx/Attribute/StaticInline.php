<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/**
 * Marks a generated method that maps to a C `static inline` function. Such a
 * function has no exported symbol, so it cannot be bound through FFI; the method
 * exists for discoverability and throws an
 * {@see \Pnlx\Exception\UnsupportedNativeFunctionException} when called.
 */
#[Attribute(Attribute::TARGET_METHOD)]
final class StaticInline
{
}
