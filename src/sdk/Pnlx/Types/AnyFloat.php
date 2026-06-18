<?php

declare(strict_types=1);

namespace Pnlx\Types;

/** A C floating-point value. */
interface AnyFloat extends ValueInterface, \Stringable
{
    public function toFloat(): float;
}
