<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use Pnlx\Exception\FFIUnavailableException;

/**
 * Guards that the PHP FFI runtime needed to load native libraries is present.
 *
 * Invoked early (via {@see \Pnlx\Verifier::shouldEnabledFFI()} from
 * {@see \Pnlx\Runtime}) so the SDK fails fast with a clear message instead of a
 * fatal error deep inside an FFI call.
 */
class FFIVerifier
{
    /**
     * Assert that the FFI extension is loaded and not disabled via `ffi.enable=0`.
     *
     * @throws FFIUnavailableException When FFI is missing or disabled.
     */
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
