<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Exception\NativeFunctionCallException;
use Throwable;

/**
 * Thin wrapper around an FFI-loaded native library for one extension.
 *
 * Holds an open {@see FFI} handle bound to the extension's `.dylib`/`.so` plus a
 * map from PHP-facing function names to native symbols, so callers
 * can invoke native functions by their ergonomic alias. Built by {@see load()}
 * and handed out from {@see \Pnlx\Runtime::native()}.
 */
class NativeLibrary
{
    /**
     * @param FFI                    $ffi          Handle bound to the native library.
     * @param array<string, string>  $aliases      Map of PHP-facing name => native symbol.
     * @param list<FFI>              $dependencies Handles to co-loaded dependency
     *                                             libraries, kept so they stay resident
     *                                             for the lifetime of this library.
     */
    public function __construct(
        private readonly FFI $ffi,
        private readonly array $aliases,
        private readonly array $dependencies = [],
    ) {
    }

    /**
     * Open a native library from its generated CDEF, library, and alias files.
     *
     * The aliases file returns a name map and the CDEF file returns the C header
     * string passed to {@see FFI::cdef()}; both are produced by the pnl generator.
     *
     * @throws ExtensionLoadException When any input file is missing or returns the wrong type.
     *
     * @param list<string> $dependencyLibraries Absolute paths of extra shared
     *        libraries to co-load first, so the package's calls into them resolve
     *        (a `dependencies` `library_names` set, e.g. gsl -> cblas).
     */
    public static function load(string $cdefPath, string $libraryPath, string $aliasesPath, bool $requireLibraryFile = true, array $dependencyLibraries = []): self
    {
        if (!is_file($cdefPath)) {
            throw new ExtensionLoadException(sprintf('CDEF file %s does not exist.', $cdefPath));
        }
        if ($requireLibraryFile && !is_file($libraryPath)) {
            throw new ExtensionLoadException(sprintf('Native library %s does not exist.', $libraryPath));
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

        $normalizedAliases = self::normalizeAliases($aliases);

        // The common case: a single library. Bind the cdef straight to it so its
        // symbols resolve, matching the original behaviour exactly.
        if ($dependencyLibraries === []) {
            return new self(FFI::cdef($cdef, $libraryPath), $normalizedAliases);
        }

        // Multi-library: load every dependency and the package's own library into the
        // global symbol table (an empty cdef dlopen()s with global visibility), then
        // bind the one monolithic cdef with NO library so each declared function
        // resolves against whichever loaded library exports it — exactly like a C
        // program linked against several libraries. It stays a single FFI scope, so a
        // value allocated for one library's call can be passed to another's. The
        // handles are kept so the libraries are not unloaded.
        //
        // Dependencies load FIRST, then the package's own library, because a dynamic
        // linker that resolves eagerly (musl/Alpine, and ELF with -z now) binds the
        // package library's undefined symbols at load time — they must already be
        // present (e.g. gsl needs cblas, which it does not itself link).
        $loaded = [];
        foreach (array_merge($dependencyLibraries, [$libraryPath]) as $path) {
            if (!is_string($path) || $path === '' || !is_file($path)) {
                continue;
            }
            try {
                $loaded[] = FFI::cdef('', $path);
            } catch (Throwable $e) {
                throw new ExtensionLoadException(
                    sprintf('Failed to co-load library %s.', $path),
                    0,
                    $e
                );
            }
        }

        return new self(FFI::cdef($cdef), $normalizedAliases, $loaded);
    }

    /**
     * Invoke a native function by its PHP-facing name.
     *
     * @param string      $name      Alias or native symbol name to call.
     * @param list<mixed> $arguments Positional arguments forwarded to the native function.
     * @return mixed The native function's return value.
     */
    public function call(string $name, array $arguments): mixed
    {
        // PHP-facing snake/camel/pascal names all resolve to the generated native symbol map.
        $native = $this->aliases[$name] ?? $this->aliases[strtolower($name)] ?? $name;

        try {
            return $this->ffi->{$native}(...$arguments);
        } catch (Throwable $e) {
            throw new NativeFunctionCallException(
                sprintf('Native function %s could not be called.', $native),
                0,
                $e
            );
        }
    }

    /**
     * Allocate a C value in this library's own FFI scope.
     *
     * Use this for a *package* type a library-less scope can't size (a generated
     * struct like libconfig's `config_t`, allocated via `new ...\Types\config_t()`).
     * The value lives in the same scope as the loaded library, so it can be passed
     * straight to its functions.
     *
     * @throws ExtensionLoadException When FFI cannot allocate the requested type.
     */
    public function allocate(string $type): CData
    {
        $value = $this->ffi->new($type);
        if ($value === null) {
            throw new ExtensionLoadException(sprintf('Failed to allocate C value of type %s.', $type));
        }

        return $value;
    }

    /**
     * The address of an exported global variable (a pointer to it), for an API that
     * takes a pointer to a global the typed function bindings can't reach — e.g.
     * oniguruma's `ONIG_ENCODING_UTF8`, which is `&OnigEncodingUTF8`. A global the
     * cdef doesn't declare raises an `\FFI\Exception`.
     *
     * @throws ExtensionLoadException When the global is not a pointer-addressable value.
     */
    /**
     * The value of an exported global variable (oniguruma's `OnigDefaultSyntax`,
     * itself a pointer). Use {@see addressOf()} instead when the API wants a pointer
     * *to* the global. A global the cdef doesn't declare raises an `\FFI\Exception`.
     */
    public function global(string $name): mixed
    {
        return $this->ffi->{$name};
    }

    public function addressOf(string $global): CData
    {
        $value = $this->ffi->{$global};
        if (!$value instanceof CData) {
            throw new ExtensionLoadException(
                sprintf('Global variable %s is not a pointer-addressable value.', $global),
            );
        }

        return FFI::addr($value);
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
