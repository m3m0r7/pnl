<?php

declare(strict_types=1);

namespace Pnlx\Attribute;

use Attribute;

/** The package version of the native library a generated extension class binds to. */
#[Attribute(Attribute::TARGET_CLASS)]
final class NativeLibraryVersion
{
    public function __construct(public readonly string $version)
    {
    }
}
