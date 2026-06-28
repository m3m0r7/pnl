<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Default {@see WorkspaceRepositoryInterface} backed by a {@see JsonReaderInterface}.
 *
 * Resolves manifest/pathmap locations through the {@see RuntimeConfigInterface} and
 * reads each file (schema-validated) via the injected JSON reader. When `pnl.json`
 * is absent it returns a minimal synthetic manifest so the SDK works before `pnl init`.
 */
class WorkspaceRepository implements WorkspaceRepositoryInterface
{
    public function __construct(
        private readonly RuntimeConfigInterface $config,
        private readonly JsonReaderInterface $jsonReader,
    ) {
    }

    /**
     * @return array<string, mixed> Decoded `pnl.json`, or a minimal default when it is absent.
     */
    public function pnlManifest(): array
    {
        $path = $this->config->projectRoot() . '/' . $this->config->pnlManifestFile();
        if (!is_file($path)) {
            // Tests and generated entrypoints can run before `pnl init`; keep a minimal workspace.
            return [
                'repositories' => [
                    ['type' => 'file', 'url' => 'file://packages'],
                ],
                'library_paths' => [],
                'extensions' => [],
            ];
        }

        return $this->jsonReader->read($path, 'pnl');
    }

    /**
     * @return array<string, mixed> Decoded workspace pathmap.
     */
    public function pathmap(): array
    {
        return $this->jsonReader->read($this->config->projectRoot() . '/' . $this->config->pathmapFile(), 'pnlx-pathmap');
    }

    /**
     * @param string $extensionRoot Directory containing the extension's `pnlx.json`.
     * @return array<string, mixed> Decoded `pnlx.json` manifest.
     */
    public function pnlxManifest(string $extensionRoot): array
    {
        return $this->jsonReader->read($extensionRoot . '/' . $this->config->pnlxManifestFile(), 'pnlx');
    }
}
