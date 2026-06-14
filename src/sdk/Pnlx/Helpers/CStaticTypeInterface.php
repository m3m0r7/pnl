<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/** A value that can be handed to a native FFI call. */
interface CStaticTypeInterface
{
    /** The value passed to the native function (a PHP scalar or `\FFI\CData`). */
    public function toValue(): mixed;
}
