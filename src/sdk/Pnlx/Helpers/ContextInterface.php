<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/** A wrapped FFI pointer/handle. */
interface ContextInterface extends CStaticTypeInterface
{
    public function cdata(): \FFI\CData;
}
