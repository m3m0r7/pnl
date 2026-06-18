<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/** The original C-library symbol name a generated method/function wraps. */
#[Attribute(Attribute::TARGET_METHOD | Attribute::TARGET_FUNCTION)]
final class RawNativeName
{
    public function __construct(public readonly string $name)
    {
    }
}
