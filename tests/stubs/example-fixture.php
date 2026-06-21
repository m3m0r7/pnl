<?php

declare(strict_types=1);

/*
 * PHPStan stub for the example test fixture's GENERATED classes.
 *
 * `RuntimeWorkspace` installs the `example` package into a temporary directory at
 * test time, so its generated wrappers (`Pnlx\Example\Types\example_point`, …) are
 * never on PHPStan's analysis path even though they exist at runtime. This file
 * declares their shape — mirroring `tests/golden/example/` — so the runtime tests
 * can use the natural typed API (`$point->getX()`) and still analyse cleanly.
 *
 * It is loaded by PHPStan's `scanFiles` only (never required at runtime), so it does
 * not collide with the real generated classes. Keep it in sync with the generator's
 * struct-accessor output.
 */

namespace Pnlx\Example\Types;

class example_point
{
    public function __construct(?\FFI\CData $cdata = null, int $size = 1)
    {
    }

    public function getX(): int
    {
    }

    public function setX(int $value): static
    {
    }

    public function getY(): int
    {
    }

    public function setY(int $value): static
    {
    }
}
