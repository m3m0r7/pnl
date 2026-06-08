<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

class RuntimeConfig implements RuntimeConfigInterface
{
    private string $projectRoot;

    public function __construct(?string $projectRoot = null)
    {
        $this->projectRoot = $this->normalizeDirectory($projectRoot ?? getcwd());
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

    public function pathmapFile(): string
    {
        return '@pnlx/pnlx-pathmap.json';
    }

    public function generatedDir(): string
    {
        return 'src/generated';
    }

    public function aliasesFile(): string
    {
        return 'function.aliases.php';
    }

    private function normalizeDirectory(string $path): string
    {
        $realpath = realpath($path);

        if ($realpath === false || !is_dir($realpath)) {
            throw new ExtensionLoadException(sprintf('Project root %s does not exist.', $path));
        }

        return $realpath;
    }
}
