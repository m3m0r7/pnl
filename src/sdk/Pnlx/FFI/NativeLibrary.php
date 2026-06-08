<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use Pnlx\Exception\ExtensionLoadException;

class NativeLibrary
{
    /**
     * @param array<string, string> $aliases
     */
    public function __construct(
        private readonly FFI $ffi,
        private readonly array $aliases,
    ) {
    }

    public static function load(string $cdefPath, string $libraryPath, string $aliasesPath): self
    {
        if (!is_file($cdefPath)) {
            throw new ExtensionLoadException(sprintf('CDEF file %s does not exist.', $cdefPath));
        }
        if (!is_file($libraryPath)) {
            throw new ExtensionLoadException(sprintf('Native bridge %s does not exist.', $libraryPath));
        }
        if (!is_file($aliasesPath)) {
            throw new ExtensionLoadException(sprintf('Aliases file %s does not exist.', $aliasesPath));
        }

        $aliases = require $aliasesPath;
        if (!is_array($aliases)) {
            throw new ExtensionLoadException(sprintf('Aliases file %s must return an array.', $aliasesPath));
        }

        return new self(
            FFI::cdef(require $cdefPath, $libraryPath),
            self::normalizeAliases($aliases)
        );
    }

    /**
     * @param list<mixed> $arguments
     */
    public function call(string $name, array $arguments): mixed
    {
        // PHP-facing snake/camel/pascal names all resolve to the generated bridge symbol map.
        $native = $this->aliases[$name] ?? $this->aliases[strtolower($name)] ?? $name;

        return $this->ffi->{$native}(...$arguments);
    }

    /**
     * @param array<mixed> $aliases
     * @return array<string, string>
     */
    private static function normalizeAliases(array $aliases): array
    {
        $normalized = [];
        foreach ($aliases as $alias => $native) {
            if (!is_string($alias) || !is_string($native)) {
                throw new ExtensionLoadException('Aliases must be a string map.');
            }
            $normalized[$alias] = $native;
        }

        return $normalized;
    }
}
