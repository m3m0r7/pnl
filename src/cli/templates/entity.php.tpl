<?php

declare(strict_types=1);

/*
 * This file is generated. Manual edits may be overwritten.
 * To customize behavior, add an overriding class under src/ and implement your methods there.
 *
 * Generated at: {{GENERATED_AT}}
 * Generated on: {{GENERATED_HOST}}
 * Generator OS: {{GENERATED_OS}}
 * PHP version: {{GENERATED_PHP_VERSION}}
 */

namespace {{NAMESPACE}};

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\NativeLibrary;
use Pnlx\RuntimeInterface;

class {{CLASS}}
{
    protected NativeLibrary $native;

    protected RuntimeInterface $runtime;

    protected {{CLASS}}Context $context;

    public function __construct(RuntimeInterface $runtime)
    {
        $this->runtime = $runtime;
        $context = $runtime->context(self::class);
        if (!$context instanceof {{CLASS}}Context) {
            throw new ExtensionLoadException(sprintf(
                'Extension context for %s must be an instance of %s.',
                self::class,
                {{CLASS}}Context::class
            ));
        }
        $this->context = $context;

        if (!is_file($this->context->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }

        $this->native = $runtime->native(self::class, '{{FFI_FILE}}');
    }

    public function __call(string $name, array $arguments): mixed
    {
        return $this->native->call($name, $arguments);
    }

{{METHODS}}
}
