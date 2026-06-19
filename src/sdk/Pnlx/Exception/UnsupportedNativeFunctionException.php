<?php

declare(strict_types=1);

namespace Pnlx\Exception;

/**
 * Raised when a generated method that has no FFI binding is called — today a C
 * `static inline` function, which has no exported symbol to dispatch to. The
 * method is still generated (so the API surface is complete and discoverable);
 * invoking it throws this instead of failing with "method not found".
 */
class UnsupportedNativeFunctionException extends PHPNativeLibraryException
{
}
