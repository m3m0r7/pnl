<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Resolves the conventional filenames and directories of a pnl workspace.
 *
 * Centralises the project root, the manifest filenames (`pnl.json`/`pnlx.json`),
 * and the configurable `output_dir` so the rest of the SDK
 * ({@see WorkspaceRepository}, {@see ExtensionRegistry}, {@see Runtime}) does not
 * hard-code workspace layout.
 */
interface RuntimeConfigInterface
{
    /** Absolute, real path of the project root. */
    public function projectRoot(): string;

    /** Filename of the top-level workspace manifest (`pnl.json`). */
    public function pnlManifestFile(): string;

    /** Filename of a per-extension manifest (`pnlx.json`). */
    public function pnlxManifestFile(): string;

    /** Configured output directory (from `pnl.json`'s `output_dir`, default `@pnlx`). */
    public function outputDir(): string;

    /** Path (relative to the project root) of the pathmap file under the output dir. */
    public function pathmapFile(): string;

    /** Directory (relative to an extension root) holding generated PHP sources. */
    public function generatedDir(): string;

    /** Filename of the generated FFI function-alias map. */
    public function aliasesFile(): string;
}
