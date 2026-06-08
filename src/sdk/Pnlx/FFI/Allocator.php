<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;
use Pnlx\Exception\ExtensionLoadException;

class Allocator
{
    private FFI $ffi;

    public function __construct()
    {
        $this->ffi = FFI::cdef('');
    }

    public function new(string $type): CData
    {
        return $this->ffi->new($type);
    }

    public function voidPointerArray(int $length): CData
    {
        if ($length < 1) {
            throw new ExtensionLoadException('Pointer array length must be greater than zero.');
        }

        return $this->new(sprintf('void *[%d]', $length));
    }
}
