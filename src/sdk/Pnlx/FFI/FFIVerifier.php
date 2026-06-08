<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use Pnlx\Exception\FFIUnavailableException;

class FFIVerifier
{
    public function shouldBeEnabled(): void
    {
        if (!class_exists(FFI::class)) {
            throw new FFIUnavailableException('PHP FFI extension is not loaded.');
        }

        if (ini_get('ffi.enable') === '0') {
            throw new FFIUnavailableException('PHP FFI is disabled by ffi.enable.');
        }
    }
}
