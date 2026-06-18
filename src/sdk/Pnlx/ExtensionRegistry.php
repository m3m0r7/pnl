<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Discovers installed extensions on disk and exposes them as definitions.
 *
 * Given an extension class name, it walks every candidate root (project root,
 * installed packages under the configured output dir, and `file` repositories
 * declared in `pnl.json`), reads each `pnlx.json` via the
 * {@see WorkspaceRepositoryInterface}, and returns the matching
 * {@see ExtensionDefinition}. Resolved definitions are memoised. Collaborates
 * with {@see Runtime} which uses it to load entrypoints and native libraries.
 */
class ExtensionRegistry implements ExtensionRegistryInterface
{
    /** @var array<string, ExtensionDefinition> Resolved definitions keyed by requested class name. */
    private array $definitions = [];

    public function __construct(
        private readonly RuntimeConfigInterface $config,
        private readonly WorkspaceRepositoryInterface $repository,
    ) {
    }

    /**
     * Resolve the installed extension whose manifest declares the given class.
     *
     * @throws ExtensionLoadException When no candidate root contains a matching manifest.
     */
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

    /**
     * Include the extension's entrypoint file so its generated classes are defined.
     *
     * @throws ExtensionLoadException When the manifest lacks an `entrypoint` or it does not exist on disk.
     */
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
     * Collect every directory that may contain an extension manifest, in priority order.
     *
     * Order matters: the project root and installed packages are scanned first
     * (installed packages are authoritative at runtime), then `file` repository
     * roots act as a fallback. Results are de-duplicated by realpath.
     *
     * @return list<string> Unique, existing absolute directory paths.
     */
    private function candidateExtensionRoots(): array
    {
        $roots = [];
        if (is_file($this->config->projectRoot() . '/' . $this->config->pnlxManifestFile())) {
            $roots[] = $this->config->projectRoot();
        }

        // Installed packages are authoritative at runtime; repository roots are only a fallback.
        // Layout: <output>/packages/<vendor>/<package>/<version>/pnlx.json.
        $packagesGlob = $this->config->projectRoot() . '/' . $this->config->outputDir()
            . '/packages/*/*/*/' . $this->config->pnlxManifestFile();
        foreach (glob($packagesGlob) ?: [] as $manifestPath) {
            $roots[] = dirname($manifestPath);
        }

        $manifest = $this->repository->pnlManifest();
        $repositories = $manifest['repositories'] ?? [];
        if (!is_array($repositories)) {
            $repositories = [];
        }

        foreach ($repositories as $repository) {
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

        // De-duplicate by realpath so the same root reached via different paths is scanned once.
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
     * Derive the effective extension class name from a manifest.
     *
     * Applies the optional `class_prefix` to the final segment of the namespaced
     * `class`, so e.g. prefix `My` turns `Vendor\Pkg\Thing` into `Vendor\Pkg\MyThing`.
     *
     * @param array<string, mixed> $manifest
     * @throws ExtensionLoadException When the manifest is missing a string `class`.
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

    /**
     * Resolve a possibly-relative repository path against the project root.
     *
     * Empty paths fall back to the project root; absolute paths are returned as-is.
     */
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
