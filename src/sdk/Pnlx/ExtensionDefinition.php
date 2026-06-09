<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Immutable value object describing one resolved, installed extension.
 *
 * Produced by {@see ExtensionRegistry::definition()} after it has located an
 * extension root, read its `pnlx.json` manifest, and computed the effective
 * extension class name. It bundles those three facts so {@see Runtime} can load
 * the entrypoint and resolve generated/native paths without re-scanning.
 */
class ExtensionDefinition
{
    /**
     * @param string               $extensionRoot Absolute directory containing the extension's `pnlx.json`.
     * @param array<string, mixed> $manifest      Decoded `pnlx.json` manifest contents.
     * @param string               $class         Effective (prefix-applied) extension class name.
     */
    public function __construct(
        private readonly string $extensionRoot,
        private readonly array $manifest,
        private readonly string $class,
    ) {
    }

    /** Absolute path to the directory holding the extension's manifest and generated sources. */
    public function extensionRoot(): string
    {
        return $this->extensionRoot;
    }

    /**
     * @return array<string, mixed> The decoded `pnlx.json` manifest.
     */
    public function manifest(): array
    {
        return $this->manifest;
    }

    /** Effective extension class name (with any `class_prefix` already applied). */
    public function class(): string
    {
        return $this->class;
    }
}
