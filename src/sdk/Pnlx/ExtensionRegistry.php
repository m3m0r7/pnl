<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

class ExtensionRegistry implements ExtensionRegistryInterface
{
    /** @var array<string, ExtensionDefinition> */
    private array $definitions = [];

    public function __construct(
        private readonly RuntimeConfigInterface $config,
        private readonly WorkspaceRepositoryInterface $repository,
    ) {
    }

    public function definition(string $class): ExtensionDefinition
    {
        if (isset($this->definitions[$class])) {
            return $this->definitions[$class];
        }

        foreach ($this->candidateExtensionRoots() as $extensionRoot) {
            $manifestPath = $extensionRoot . '/' . $this->config->pnlxManifestFile();
            if (!is_file($manifestPath)) {
                continue;
            }

            $manifest = $this->repository->pnlxManifest($extensionRoot);
            $manifestClass = $this->manifestClass($manifest);
            if ($manifestClass === $class) {
                return $this->definitions[$class] = new ExtensionDefinition(
                    $extensionRoot,
                    $manifest,
                    $manifestClass
                );
            }
        }

        throw new ExtensionLoadException(sprintf('Extension class %s is not installed.', $class));
    }

    public function loadEntrypoint(string $class): void
    {
        $definition = $this->definition($class);
        $manifest = $definition->manifest();

        if (!isset($manifest['entrypoint']) || !is_string($manifest['entrypoint'])) {
            throw new ExtensionLoadException($this->config->pnlxManifestFile() . ' is missing entrypoint.');
        }

        $entrypoint = $definition->extensionRoot() . '/' . $manifest['entrypoint'];
        if (!is_file($entrypoint)) {
            throw new ExtensionLoadException(sprintf('Entrypoint %s does not exist.', $entrypoint));
        }

        require_once $entrypoint;
    }

    /**
     * @return list<string>
     */
    private function candidateExtensionRoots(): array
    {
        $roots = [];
        if (is_file($this->config->projectRoot() . '/' . $this->config->pnlxManifestFile())) {
            $roots[] = $this->config->projectRoot();
        }

        // Installed packages are authoritative at runtime; repository roots are only a fallback.
        foreach (glob($this->config->projectRoot() . '/@pnlx/packages/*/*/' . $this->config->pnlxManifestFile()) ?: [] as $manifestPath) {
            $roots[] = dirname($manifestPath);
        }

        $manifest = $this->repository->pnlManifest();
        foreach (($manifest['repositories'] ?? []) as $repository) {
            if (!is_array($repository) || ($repository['type'] ?? null) !== 'file') {
                continue;
            }

            $url = $repository['url'] ?? '';
            if (!is_string($url)) {
                continue;
            }

            $path = str_starts_with($url, 'file://') ? substr($url, strlen('file://')) : $url;
            $repositoryRoot = $this->absolutePath($path);
            foreach (glob($repositoryRoot . '/*/' . $this->config->pnlxManifestFile()) ?: [] as $manifestPath) {
                $roots[] = dirname($manifestPath);
            }
        }

        $normalized = [];
        foreach ($roots as $root) {
            $realpath = realpath($root);
            if ($realpath !== false && is_dir($realpath)) {
                $normalized[$realpath] = $realpath;
            }
        }

        return array_values($normalized);
    }

    /**
     * @param array<string, mixed> $manifest
     */
    private function manifestClass(array $manifest): string
    {
        if (!isset($manifest['class']) || !is_string($manifest['class'])) {
            throw new ExtensionLoadException($this->config->pnlxManifestFile() . ' is missing class.');
        }

        $class = $manifest['class'];
        $prefix = isset($manifest['class_prefix']) && is_string($manifest['class_prefix'])
            ? $manifest['class_prefix']
            : '';

        if ($prefix === '') {
            return $class;
        }

        $separator = strrpos($class, '\\');
        if ($separator === false) {
            return $prefix . $class;
        }

        return substr($class, 0, $separator + 1) . $prefix . substr($class, $separator + 1);
    }

    private function absolutePath(string $path): string
    {
        if ($path === '') {
            return $this->config->projectRoot();
        }

        if (str_starts_with($path, '/')) {
            return $path;
        }

        return $this->config->projectRoot() . '/' . $path;
    }
}
