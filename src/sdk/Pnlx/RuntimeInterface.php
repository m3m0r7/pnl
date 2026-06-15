<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\FFI\Allocator;

/**
 * Public entry point for loading pnl-generated extensions.
 *
 * A {@see Runtime} resolves an extension class to its installed package and loads
 * its generated PHP entrypoint. Generated entities are pure static and boot
 * themselves from their baked constants, so the runtime is mainly used to load
 * entrypoints and read manifests.
 */
interface RuntimeInterface
{
    /**
     * Load (and boot) the extension class's static entrypoint. Entities are pure
     * static and are never instantiated.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When the class is not defined after loading its entrypoint.
     */
    public function loadEntrypoint(string $class): void;

    /**
     * Load the extension and return its generated `*Manifest` describing the native bridge.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When the info class is missing or not an {@see ManifestInterface}.
     */
    public function loadManifest(string $class): ManifestInterface;

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

    /** Shared {@see Allocator} for creating standalone FFI C data. */
    public function allocator(): Allocator;
}
