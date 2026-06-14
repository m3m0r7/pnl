<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Read-only view over a single installed extension's compiled native bridge.
 *
 * A concrete `*Manifest` class is emitted alongside each generated extension and
 * exposed through the entity's readonly `$manifest` field. It exposes the extension's identity
 * metadata together with the filesystem path of the compiled `.dylib`/`.so`
 * bridge that {@see Runtime::native()} hands to {@see FFI\NativeLibrary::load()}.
 */
interface ManifestInterface
{
    /** Fully-qualified extension class name this info describes. */
    public function name(): string;

    /** Semantic version of the installed extension package. */
    public function version(): string;

    /** Content hash identifying the exact build of the extension. */
    public function hash(): string;

    /** Human-readable description taken from the extension manifest. */
    public function description(): string;

    /** Absolute path to the compiled native bridge library (`.dylib`/`.so`). */
    public function path(): string;
}
