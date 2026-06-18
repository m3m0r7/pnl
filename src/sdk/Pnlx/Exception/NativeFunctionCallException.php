<?php

declare(strict_types=1);

namespace Pnlx\Exception;

/** Raised when PHP FFI rejects a generated native function call. */
class NativeFunctionCallException extends PHPNativeLibraryException
{
}
