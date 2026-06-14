<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Default {@see RuntimeConfigInterface} resolving paths from the project root.
 *
 * The project root defaults to the current working directory and is normalised to
 * a real, existing directory. The `output_dir` is read once (and memoised) from
 * `pnl.json`, falling back to `@pnlx` when the manifest is absent or omits it.
 */
class RuntimeConfig implements RuntimeConfigInterface
{
    private string $projectRoot;

    /** Memoised output dir; null until first resolved by {@see outputDir()}. */
    private ?string $outputDir = null;

    /**
     * @param string|null $projectRoot Project root, or null to use the current working directory.
     * @throws ExtensionLoadException When the working directory cannot be determined or the root does not exist.
     */
    public function __construct(?string $projectRoot = null)
    {
        if ($projectRoot === null) {
            // Prefer the pnl.json that `@pnlx/autoload.php` resolved from its own
            // location (cwd-independent and move-safe); fall back to cwd.
            $manifest = defined('PNLX_PROJECT_MANIFEST') ? constant('PNLX_PROJECT_MANIFEST') : null;
            if (is_string($manifest) && is_file($manifest)) {
                $projectRoot = dirname($manifest);
            } else {
                $cwd = getcwd();
                if ($cwd === false) {
                    throw new ExtensionLoadException('Unable to determine the current working directory.');
                }
                $projectRoot = $cwd;
            }
        }

        $this->projectRoot = $this->normalizeDirectory($projectRoot);
    }

    public function projectRoot(): string
    {
        return $this->projectRoot;
    }

    public function pnlManifestFile(): string
    {
        return 'pnl.json';
    }

    public function pnlxManifestFile(): string
    {
        return 'pnlx.json';
    }

    /**
     * Resolve the configured output directory, reading `pnl.json` at most once.
     *
     * Defaults to `@pnlx`; overridden by a non-empty string `output_dir` in the manifest.
     */
    public function outputDir(): string
    {
        if ($this->outputDir !== null) {
            return $this->outputDir;
        }

        $outputDir = '@pnlx';
        $manifestPath = $this->projectRoot . '/' . $this->pnlManifestFile();
        if (is_file($manifestPath)) {
            $raw = file_get_contents($manifestPath);
            if ($raw !== false) {
                $data = json_decode($raw, true);
                if (
                    is_array($data)
                    && isset($data['output_dir'])
                    && is_string($data['output_dir'])
                    && $data['output_dir'] !== ''
                    && $this->isSafeRelativePath($data['output_dir'])
                ) {
                    $outputDir = $data['output_dir'];
                }
            }
        }

        return $this->outputDir = $outputDir;
    }

    private function isSafeRelativePath(string $path): bool
    {
        if ($path === '' || str_starts_with($path, '/') || str_contains($path, '\\')) {
            return false;
        }

        foreach (explode('/', $path) as $segment) {
            if ($segment === '..') {
                return false;
            }
        }

        return true;
    }

    public function pathmapFile(): string
    {
        return $this->outputDir() . '/pnlx-pathmap.json';
    }

    public function generatedDir(): string
    {
        return 'src/generated';
    }

    public function aliasesFile(): string
    {
        return 'function.aliases.php';
    }

    /**
     * Resolve a path to its real, existing directory form.
     *
     * @throws ExtensionLoadException When the path does not resolve to an existing directory.
     */
    private function normalizeDirectory(string $path): string
    {
        $realpath = realpath($path);

        if ($realpath === false || !is_dir($realpath)) {
            throw new ExtensionLoadException(sprintf('Project root %s does not exist.', $path));
        }

        return $realpath;
    }
}
