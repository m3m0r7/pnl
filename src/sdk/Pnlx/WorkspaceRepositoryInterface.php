<?php

declare(strict_types=1);

namespace Pnlx;

interface WorkspaceRepositoryInterface
{
    /**
     * @return array<string, mixed>
     */
    public function pnlManifest(): array;

    /**
     * @return array<string, mixed>
     */
    public function pathmap(): array;

    /**
     * @return array<string, mixed>
     */
    public function pnlxManifest(string $extensionRoot): array;
}
