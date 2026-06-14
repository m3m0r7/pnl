<?php

declare(strict_types=1);

namespace Pnlx\Exception;

use RuntimeException;

/**
 * Root of every exception raised by the pnl native-library runtime.
 *
 * Both the SDK's own failure types (e.g. {@see ExtensionLoadException}) and the
 * per-extension exceptions emitted by the generator (e.g. `Pnlx\Example\ExampleException`)
 * extend this class, so callers can catch any pnl failure with a single
 * `catch (PHPNativeLibraryException $e)` while still narrowing to a specific
 * extension or failure mode when they need to.
 */
class PHPNativeLibraryException extends RuntimeException
{
}
