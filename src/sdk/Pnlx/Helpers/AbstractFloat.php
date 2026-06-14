<?php

declare(strict_types=1);

namespace Pnlx\Helpers;

abstract class AbstractFloat implements AnyFloat
{
    private readonly float $value;

    public function __construct(float|string $value)
    {
        $this->value = (float) $value;
    }

    public function __toString(): string
    {
        return (string) $this->value;
    }

    public function toFloat(): float
    {
        return $this->value;
    }

    public function toValue(): float
    {
        return $this->value;
    }
}
