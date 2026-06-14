<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

/** A C string (`char *`), usable anywhere a PHP string is. */
class String_ implements CStaticTypeInterface, \Stringable
{
    public function __construct(private readonly string $value)
    {
    }

    public function __toString(): string
    {
        return $this->value;
    }

    public function toValue(): string
    {
        return $this->value;
    }
}
