<?php

declare(strict_types=1);

namespace Pnlx\Types;

/** A value that can be handed to a native FFI call. */
interface ValueInterface
{
    /** The value passed to the native function (a PHP scalar or `\FFI\CData`). */
    public function toValue(): mixed;
}
