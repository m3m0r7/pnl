<?php

declare(strict_types=1);

namespace Pnlx\Exception;

/**
 * Thrown whenever the SDK loader cannot locate, validate, or load an extension.
 *
 * This is the SDK's load-time failure type: missing manifests, malformed JSON,
 * schema-validation failures, absent native library files, and unresolved
 * extension classes all surface as this exception (or a subclass of it). It
 * specialises {@see PHPNativeLibraryException} so a single catch can cover every
 * pnl runtime failure, including the per-extension exceptions the generator emits.
 */
class ExtensionLoadException extends PHPNativeLibraryException
{
}
