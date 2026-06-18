<?php

declare(strict_types=1);

namespace Pnlx\Exception;

/**
 * Raised when the PHP FFI extension required to load native libraries is missing
 * or disabled (`ffi.enable=0`).
 *
 * Specialises {@see ExtensionLoadException} so callers can distinguish an
 * unusable FFI environment from ordinary extension-loading failures while still
 * catching both via the shared base type. Thrown by {@see \Pnlx\FFI\FFIVerifier}.
 */
class FFIUnavailableException extends ExtensionLoadException
{
}
