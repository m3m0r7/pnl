<?php

declare(strict_types=1);

namespace Pnlx\Exception;

use RuntimeException;

/**
 * Thrown whenever the SDK cannot locate, validate, or load an extension.
 *
 * This is the SDK's general-purpose failure type: missing manifests, malformed
 * JSON, schema-validation failures, absent native bridge files, and unresolved
 * extension classes all surface as this exception (or a subclass of it).
 */
class ExtensionLoadException extends RuntimeException
{
}
