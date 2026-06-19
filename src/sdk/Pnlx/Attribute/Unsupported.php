<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/**
 * Marks a generated method that cannot be bound through FFI for a reason without a
 * more specific marker. The method exists for discoverability and throws an
 * {@see \Pnlx\Exception\UnsupportedNativeFunctionException} when called.
 */
#[Attribute(Attribute::TARGET_METHOD)]
final class Unsupported
{
}
