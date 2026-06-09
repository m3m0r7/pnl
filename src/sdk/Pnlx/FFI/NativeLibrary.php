<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use Pnlx\Exception\ExtensionLoadException;

/**
 * Thin wrapper around an FFI-loaded compiled bridge for one extension.
 *
 * Holds an open {@see FFI} handle bound to the extension's `.dylib`/`.so` plus a
 * map from PHP-facing function names to the bridge's exported symbols, so callers
 * can invoke native functions by their ergonomic alias. Built by {@see load()}
 * and handed out from {@see \Pnlx\Runtime::native()}.
 */
class NativeLibrary
{
    /**
     * @param FFI                    $ffi     Handle bound to the compiled bridge library.
     * @param array<string, string>  $aliases Map of PHP-facing name => native bridge symbol.
     */
    public function __construct(
        private readonly FFI $ffi,
        private readonly array $aliases,
    ) {
    }

    /**
     * Open a compiled bridge from its generated CDEF, library, and alias files.
     *
     * The aliases file returns a name map and the CDEF file returns the C header
     * string passed to {@see FFI::cdef()}; both are produced by the pnl generator.
     *
     * @throws ExtensionLoadException When any input file is missing or returns the wrong type.
     */
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

        $cdef = require $cdefPath;
        if (!is_string($cdef)) {
            throw new ExtensionLoadException(sprintf('CDEF file %s must return a string.', $cdefPath));
        }

        return new self(
            FFI::cdef($cdef, $libraryPath),
            self::normalizeAliases($aliases)
        );
    }

    /**
     * Invoke a native bridge function by its PHP-facing name.
     *
     * @param string      $name      Alias or native symbol name to call.
     * @param list<mixed> $arguments Positional arguments forwarded to the native function.
     * @return mixed The native function's return value.
     */
    public function call(string $name, array $arguments): mixed
    {
        // PHP-facing snake/camel/pascal names all resolve to the generated bridge symbol map.
        $native = $this->aliases[$name] ?? $this->aliases[strtolower($name)] ?? $name;

        return $this->ffi->{$native}(...$arguments);
    }

    /**
     * Validate and narrow the raw alias map to a string=>string map.
     *
     * @param array<mixed> $aliases Raw map returned by the generated aliases file.
     * @return array<string, string> Validated PHP-facing name => native symbol map.
     * @throws ExtensionLoadException When any key or value is not a string.
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
