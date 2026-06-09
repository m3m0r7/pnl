<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Reads the workspace's JSON manifests, pathmap, and per-extension manifests.
 *
 * Abstracts access to `pnl.json`, the `pnlx-pathmap.json`, and each `pnlx.json`,
 * each validated against its schema before decoding. Consumed by
 * {@see ExtensionRegistry} and {@see Runtime}.
 */
interface WorkspaceRepositoryInterface
{
    /**
     * Decoded top-level workspace manifest (`pnl.json`).
     *
     * @return array<string, mixed>
     */
    public function pnlManifest(): array;

    /**
     * Decoded workspace pathmap (`pnlx-pathmap.json`) under the output dir.
     *
     * @return array<string, mixed>
     */
    public function pathmap(): array;

    /**
     * Decoded `pnlx.json` manifest for the extension rooted at the given directory.
     *
     * @return array<string, mixed>
     */
    public function pnlxManifest(string $extensionRoot): array;
}
