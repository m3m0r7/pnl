<?php

declare(strict_types=1);

namespace Pnlx;

interface RuntimeConfigInterface
{
    public function projectRoot(): string;

    public function pnlManifestFile(): string;

    public function pnlxManifestFile(): string;

    public function pathmapFile(): string;

    public function generatedDir(): string;

    public function aliasesFile(): string;
}
