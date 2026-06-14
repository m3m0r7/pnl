<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/** A C floating-point value. */
interface AnyFloat extends CStaticTypeInterface, \Stringable
{
    public function toFloat(): float;
}
