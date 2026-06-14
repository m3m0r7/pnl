<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\FFI\FFIVerifier;

/**
 * Static facade for the SDK's pre-flight checks.
 *
 * The runtime asserts only that PHP FFI is usable ({@see FFIVerifier}) before
 * loading native bridges. OpenAPI schema validation of workspace JSON files is
 * owned by the Rust toolchain (`pnl install` / `pnl validate` validate every
 * file as it is read), so it is not repeated here and the SDK has no external
 * dependency on an OpenAPI validator.
 */
class Verifier
{
    /**
     * Assert the FFI environment can load native bridges.
     *
     * @throws \Pnlx\Exception\FFIUnavailableException When FFI is missing or disabled.
     */
    public static function shouldEnabledFFI(): void
    {
        (new FFIVerifier())->shouldBeEnabled();
    }
}
