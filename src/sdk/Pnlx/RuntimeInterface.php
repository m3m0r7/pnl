<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\FFI\Allocator;
use Pnlx\FFI\NativeLibrary;

/**
 * Public entry point for loading pnl-generated extensions and their native bridges.
 *
 * A {@see Runtime} resolves an extension class to its installed package, loads the
 * generated PHP entrypoint, and exposes the compiled native bridge via FFI. Generated
 * extension classes receive the runtime in their constructor and call back into it
 * to read manifests, resolve generated file paths, and obtain a {@see NativeLibrary}.
 */
interface RuntimeInterface
{
    /**
     * Load the extension class's entrypoint and return a new instance bound to this runtime.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When the class is not defined after loading its entrypoint.
     */
    public function load(string $class): object;

    /**
     * Load the extension and return its generated `*Manifest` describing the native bridge.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When the info class is missing or not an {@see ManifestInterface}.
     */
    public function loadManifest(string $class): ManifestInterface;

    /** Absolute directory of the installed extension that declares the given class. */
    public function extensionRoot(string $class): string;

    /** Absolute path of the project root this runtime operates within. */
    public function projectRoot(): string;

    /**
     * Decoded `pnlx.json` manifest for the given extension class.
     *
     * @return array<string, mixed>
     */
    public function manifest(string $class): array;

    /**
     * Decoded pathmap (`pnlx-pathmap.json`) for the workspace.
     *
     * @return array<string, mixed>
     */
    public function pathmap(): array;

    /** Absolute path to a file inside the extension's generated-sources directory. */
    public function generatedPath(string $class, string $file): string;

    /** Filename (relative to the generated dir) of the FFI function-alias map. */
    public function aliasesFile(): string;

    /**
     * Open the compiled native bridge for the given extension class.
     *
     * @param string $class   Extension class whose bridge to load.
     * @param string $ffiFile Generated CDEF filename describing the bridge's C API.
     * @throws \Pnlx\Exception\ExtensionLoadException When the bridge library does not exist.
     */
    public function native(string $class, string $ffiFile): NativeLibrary;

    /** Shared {@see Allocator} for creating standalone FFI C data. */
    public function allocator(): Allocator;
}
