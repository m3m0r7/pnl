<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Locates installed extensions and loads their PHP entrypoints.
 *
 * Implementations resolve an extension class name to a concrete
 * {@see ExtensionDefinition} by scanning candidate roots (project root, installed
 * packages under the output dir, and configured file repositories) and matching
 * the `pnlx.json` manifest. Consumed by {@see Runtime}.
 */
interface ExtensionRegistryInterface
{
    /**
     * Resolve an extension class name to its installed definition.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When no installed manifest declares the class.
     */
    public function definition(string $class): ExtensionDefinition;

    /**
     * `require_once` the entrypoint PHP file declared by the extension's manifest
     * so its generated classes become available.
     *
     * @throws \Pnlx\Exception\ExtensionLoadException When the manifest entrypoint is missing or absent on disk.
     */
    public function loadEntrypoint(string $class): void;
}
