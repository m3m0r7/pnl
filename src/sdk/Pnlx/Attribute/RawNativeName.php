<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/** The original C-library symbol name a generated method wraps (not the Rust bridge symbol). */
#[Attribute(Attribute::TARGET_METHOD)]
final class RawNativeName
{
    public function __construct(public readonly string $name)
    {
    }
}
