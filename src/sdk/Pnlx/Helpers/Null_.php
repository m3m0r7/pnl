<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/**
 * The null member of the value-object family: an absent value or null pointer.
 *
 * Its {@see toValue()} is `null`, so it marshals to a native NULL, and
 * {@see \Pnlx\Util\is_null()} reports it as null.
 */
final class Null_ implements CStaticTypeInterface
{
    public function toValue(): null
    {
        return null;
    }
}
