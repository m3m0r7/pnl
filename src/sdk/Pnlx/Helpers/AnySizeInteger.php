<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/** An integer of any C width/sign, kept losslessly as a 64-bit pattern. */
interface AnySizeInteger extends CStaticTypeInterface, \Stringable
{
    public function toInt(): int;
}
