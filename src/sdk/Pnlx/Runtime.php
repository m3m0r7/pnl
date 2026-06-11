<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\Allocator;
use Pnlx\FFI\NativeLibrary;

/**
 * Central entry point that wires together the SDK and loads extensions.
 *
 * On construction it verifies the FFI environment and assembles the default
 * collaborators: a {@see RuntimeConfig} (path/output-dir resolution), a
 * {@see WorkspaceRepository} (JSON manifests/lock/pathmap), and an
 * {@see ExtensionRegistry} (locating installed packages). It then loads generated
 * extension entrypoints, instantiates their classes/contexts, and exposes the
 * compiled native bridge via {@see NativeLibrary}. Each of the collaborators can
 * be injected for testing.
 */
class Runtime implements RuntimeInterface
{
    /**
     * Generated entrypoints may instantiate Runtime without an explicit root.
     * This scope keeps those nested loads tied to the caller's project root.
     */
    private static ?string $activeProjectRoot = null;

    private RuntimeConfigInterface $config;

    private WorkspaceRepositoryInterface $repository;

    private ExtensionRegistryInterface $registry;

    private ?Allocator $allocator = null;

    /**
     * @param string|null                       $projectRoot Project root to operate within; falls back to the active scope or cwd.
     * @param RuntimeConfigInterface|null        $config      Optional config override (path/output-dir resolution).
     * @param WorkspaceRepositoryInterface|null  $repository  Optional manifest/lock/pathmap repository override.
     * @param ExtensionRegistryInterface|null    $registry    Optional extension registry override.
     * @throws \Pnlx\Exception\FFIUnavailableException When PHP FFI is unavailable.
     */
    public function __construct(
        ?string $projectRoot = null,
        ?RuntimeConfigInterface $config = null,
        ?WorkspaceRepositoryInterface $repository = null,
        ?ExtensionRegistryInterface $registry = null,
    ) {
        // Fail fast if the FFI environment cannot support loading native bridges.
        Verifier::shouldEnabledFFI();

        // Prefer the caller's root, then the scope of an enclosing (generated) load, then cwd.
        $this->config = $config ?? new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $jsonReader = new JsonReader();
        $this->repository = $repository ?? new WorkspaceRepository($this->config, $jsonReader);
        $this->registry = $registry ?? new ExtensionRegistry($this->config, $this->repository);
    }

    /**
     * Whether the workspace's `pnl.json` opts into the global functions API.
     *
     * Reads `features.use_functions` from the manifest without constructing a full runtime.
     */
    public static function enableFunctions(?string $projectRoot = null): bool
    {
        $config = new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $manifest = (new WorkspaceRepository($config, new JsonReader()))->pnlManifest();
        $features = $manifest['features'] ?? [];

        return is_array($features) && ($features['use_functions'] ?? false) === true;
    }

    /**
     * Load the extension's entrypoint then instantiate the class, passing in this runtime.
     *
     * @throws ExtensionLoadException When the class is still undefined after loading.
     */
    public function load(string $class): object
    {
        $this->withProjectRootScope(fn () => $this->registry->loadEntrypoint($class));

        if (!class_exists($class)) {
            throw new ExtensionLoadException(sprintf('Extension class %s was not loaded.', $class));
        }

        return new $class($this);
    }

    /**
     * Load the extension and build its generated `<Class>Context`.
     *
     * @throws ExtensionLoadException When the context class is missing or not a {@see ContextInterface}.
     */
    public function context(string $class): ContextInterface
    {
        $this->withProjectRootScope(fn () => $this->registry->loadEntrypoint($class));

        // Generated context classes follow the `<Class>Context` naming convention.
        $contextClass = $class . 'Context';
        if (!class_exists($contextClass)) {
            throw new ExtensionLoadException(sprintf('Extension context class %s was not loaded.', $contextClass));
        }

        $context = new $contextClass($this);
        if (!$context instanceof ContextInterface) {
            throw new ExtensionLoadException(sprintf(
                'Extension context class %s must implement %s.',
                $contextClass,
                ContextInterface::class
            ));
        }

        return $context;
    }

    /** Absolute directory of the installed extension declaring the given class. */
    public function extensionRoot(string $class): string
    {
        return $this->registry->definition($class)->extensionRoot();
    }

    public function projectRoot(): string
    {
        return $this->config->projectRoot();
    }

    /**
     * @return array<string, mixed> The extension's decoded `pnlx.json` manifest.
     */
    public function manifest(string $class): array
    {
        return $this->registry->definition($class)->manifest();
    }

    /**
     * @return array<string, mixed> The decoded workspace pathmap.
     */
    public function pathmap(): array
    {
        return $this->repository->pathmap();
    }

    /** Build an absolute path to a file under the extension's generated-sources directory. */
    public function generatedPath(string $class, string $file): string
    {
        return $this->extensionRoot($class) . '/' . $this->config->generatedDir() . '/' . $file;
    }

    public function aliasesFile(): string
    {
        return $this->config->aliasesFile();
    }

    /**
     * Open the compiled native bridge for an extension via its context and generated CDEF.
     *
     * @throws ExtensionLoadException When the bridge library reported by the context is missing.
     */
    public function native(string $class, string $ffiFile): NativeLibrary
    {
        $context = $this->context($class);
        if (!is_file($context->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }
        $actualHash = hash_file('sha256', $context->path());
        if ($actualHash === false || !hash_equals($context->hash(), $actualHash)) {
            throw new ExtensionLoadException('Native bridge hash does not match the pathmap.');
        }

        return NativeLibrary::load(
            $this->generatedPath($class, $ffiFile),
            $context->path(),
            $this->generatedPath($class, $this->aliasesFile())
        );
    }

    /** Lazily create and reuse a single {@see Allocator} for this runtime. */
    public function allocator(): Allocator
    {
        return $this->allocator ??= new Allocator();
    }

    /**
     * Run a callback with this runtime's project root published as the active scope.
     *
     * Nested loads triggered by generated entrypoints (which may build a Runtime
     * without an explicit root) inherit this root, and the previous scope is
     * always restored afterwards.
     */
    private function withProjectRootScope(callable $callback): void
    {
        // Generated entrypoints are regular PHP files, so root scoping is process-local.
        $previous = self::$activeProjectRoot;
        self::$activeProjectRoot = $this->projectRoot();

        try {
            $callback();
        } finally {
            self::$activeProjectRoot = $previous;
        }
    }
}
