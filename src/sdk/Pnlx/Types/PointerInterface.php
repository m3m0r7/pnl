<?php

declare(strict_types=1);

namespace Pnlx\Types;

/** A wrapped FFI pointer/handle. */
interface PointerInterface extends ValueInterface
{
    public function cdata(): \FFI\CData;
}
