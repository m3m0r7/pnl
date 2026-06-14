<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use FFI;
use Pnlx\Exception\ExtensionLoadException;

/**
 * Validates workspace JSON against the bundled OpenAPI schemas by calling into
 * the Rust support library over FFI (`pnl_validate_json`).
 *
 * The library is the `cdylib` build of the pnl toolchain, expanded into
 * `@pnlx/runtime` by `pnl install`. This keeps schema validation owned by Rust
 * with no PHP OpenAPI dependency. When the library is unavailable (it
 * was not produced for this build) validation is skipped — the files were
 * already validated by the Rust toolchain at install time.
 */
final class SchemaValidator
{
    private const CDEF = 'char* pnl_validate_json(const char*, const char*); void pnl_string_free(char*);';

    private ?FFI $ffi = null;

    private bool $unavailable = false;

    /**
     * Exported C function names. Held as `string` (not inline literals) so static
     * analysis treats the FFI calls below as dynamic — FFI resolves them at the
     * C ABI level, where they are not visible to PHPStan.
     */
    private string $validateFn = 'pnl_validate_json';

    private string $freeFn = 'pnl_string_free';

    public function __construct(private readonly string $libraryPath)
    {
    }

    /**
     * @throws ExtensionLoadException When the file cannot be read or fails schema validation.
     */
    public function validate(string $schema, string $path): void
    {
        $ffi = $this->ffi();
        if ($ffi === null) {
            return;
        }

        $json = file_get_contents($path);
        if ($json === false) {
            throw new ExtensionLoadException(sprintf('Failed to read %s.', $path));
        }

        // A valid document yields a NULL `char*` (PHP null); an error yields a
        // CData string to read and free.
        $error = $ffi->{$this->validateFn}($schema, $json);
        if ($error instanceof \FFI\CData) {
            $message = FFI::string($error);
            $ffi->{$this->freeFn}($error);
            throw new ExtensionLoadException(sprintf(
                '%s does not match the %s schema: %s',
                $path,
                $schema,
                $message
            ));
        }
    }

    private function ffi(): ?FFI
    {
        if ($this->ffi !== null) {
            return $this->ffi;
        }
        // Resolve to an absolute path: a relative path under `@pnlx` would start
        // with `@`, which dyld misreads as an `@rpath`-style prefix on macOS.
        $resolved = realpath($this->libraryPath);
        if ($this->unavailable || $resolved === false || !class_exists(FFI::class)) {
            $this->unavailable = true;

            return null;
        }

        return $this->ffi = FFI::cdef(self::CDEF, $resolved);
    }
}
