<?php

declare(strict_types=1);

namespace Pnlx;

interface ContextInterface
{
    public function name(): string;

    public function version(): string;

    public function hash(): string;

    public function description(): string;

    public function path(): string;
}
