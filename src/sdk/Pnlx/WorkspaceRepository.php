<?php

declare(strict_types=1);

namespace Pnlx;

class WorkspaceRepository implements WorkspaceRepositoryInterface
{
    public function __construct(
        private readonly RuntimeConfigInterface $config,
        private readonly JsonReaderInterface $jsonReader,
    ) {
    }

    public function pnlManifest(): array
    {
        $path = $this->config->projectRoot() . '/' . $this->config->pnlManifestFile();
        if (!is_file($path)) {
            // Tests and generated entrypoints can run before `pnl init`; keep a minimal workspace.
            return [
                'repositories' => [
                    ['type' => 'file', 'url' => 'file://packages'],
                ],
                'load_paths' => [],
                'extensions' => [],
            ];
        }

        return $this->jsonReader->read($path, 'pnl');
    }

    public function pathmap(): array
    {
        return $this->jsonReader->read($this->config->projectRoot() . '/' . $this->config->pathmapFile(), 'pnlx-pathmap');
    }

    public function pnlxManifest(string $extensionRoot): array
    {
        return $this->jsonReader->read($extensionRoot . '/' . $this->config->pnlxManifestFile(), 'pnlx');
    }
}
