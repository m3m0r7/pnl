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
 * extension entrypoints, instantiates their classes and manifests, and exposes the
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
        $jsonReader = new JsonReader($this->schemaValidator());
        $this->repository = $repository ?? new WorkspaceRepository($this->config, $jsonReader);
        $this->registry = $registry ?? new ExtensionRegistry($this->config, $this->repository);
        // The self-contained type layer (Pnlx\Helpers\*) ships with the SDK and is
        // resolved by the SDK autoloader, so nothing extra is loaded here.
    }

    /**
     * Build the FFI-backed schema validator pointed at the support library that
     * `pnl install` expands into `@pnlx/runtime`. It no-ops when the library is
     * absent, so workspaces without it still load (files were validated at install).
     */
    private function schemaValidator(): \Pnlx\Schema\SchemaValidator
    {
        $library = match (PHP_OS_FAMILY) {
            'Darwin' => 'libpnl.dylib',
            'Windows' => 'pnl.dll',
            default => 'libpnl.so',
        };
        $path = $this->config->projectRoot() . '/' . $this->config->outputDir() . '/runtime/' . $library;

        return new \Pnlx\Schema\SchemaValidator($path);
    }

    /**
     * Whether the workspace's `pnl.json` opts into the global functions API.
     *
     * Reads `features.use_functions` from the manifest without constructing a full runtime.
     */
    public static function enableFunctions(?string $projectRoot = null): bool
    {
        return self::feature('use_functions', $projectRoot);
    }

    /**
     * Whether the workspace's `pnl.json` opts into exposing raw `\FFI\CData` in
     * generated signatures (the `cdata/<Class>.php` entity variant).
     *
     * Reads `features.allow_cdata` from the manifest without constructing a full runtime.
     */
    public static function allowCData(?string $projectRoot = null): bool
    {
        return self::feature('allow_cdata', $projectRoot);
    }

    /**
     * Whether `features.use_php_scalars_in_params` is on, i.e. generated methods
     * accept a raw PHP scalar argument (otherwise a raw scalar throws and the
     * caller must pass a `Pnlx\Helpers\*` wrapper).
     */
    public static function useScalarsInParams(?string $projectRoot = null): bool
    {
        return self::feature('use_php_scalars_in_params', $projectRoot);
    }

    /**
     * Whether `features.use_php_scalars_in_return` is on, i.e. methods return PHP
     * native scalars (the `scalar/<Class>.php` entity variant) instead of wrappers.
     */
    public static function useScalarsInReturn(?string $projectRoot = null): bool
    {
        return self::feature('use_php_scalars_in_return', $projectRoot);
    }

    /** Read a boolean `features.*` flag from `pnl.json` without a full runtime. */
    private static function feature(string $name, ?string $projectRoot): bool
    {
        $config = new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $manifest = (new WorkspaceRepository($config, new JsonReader()))->pnlManifest();
        $features = $manifest['features'] ?? [];

        return is_array($features) && ($features[$name] ?? false) === true;
    }

    /**
     * Load (and thereby boot) the extension's static entrypoint.
     *
     * Entities are pure static and never instantiated; requiring the entrypoint
     * runs the class's one-time `initialize()` boot. Doing it inside the project
     * root scope means the entity's own `new Runtime()` resolves this runtime's
     * root even before `PNLX_PROJECT_MANIFEST` is defined.
     *
     * @throws ExtensionLoadException When the class is still undefined after loading.
     */
    public function loadEntrypoint(string $class): void
    {
        $this->withProjectRootScope(function () use ($class): void {
            $this->registry->loadEntrypoint($class);
            if (!class_exists($class)) {
                throw new ExtensionLoadException(sprintf('Extension class %s was not loaded.', $class));
            }
        });
    }

    /**
     * Load the extension and build its generated `<Class>Manifest`.
     *
     * @throws ExtensionLoadException When the manifest class is missing or not an {@see ManifestInterface}.
     */
    public function loadManifest(string $class): ManifestInterface
    {
        $this->withProjectRootScope(fn () => $this->registry->loadEntrypoint($class));

        // Generated manifest classes follow the `<Class>Manifest` naming convention.
        $manifestClass = $class . 'Manifest';
        if (!class_exists($manifestClass)) {
            throw new ExtensionLoadException(sprintf('Extension manifest class %s was not loaded.', $manifestClass));
        }

        $manifest = new $manifestClass($this);
        if (!$manifest instanceof ManifestInterface) {
            throw new ExtensionLoadException(sprintf(
                'Extension manifest class %s must implement %s.',
                $manifestClass,
                ManifestInterface::class
            ));
        }

        return $manifest;
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
        $manifest = $this->loadManifest($class);
        if (!is_file($manifest->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }
        $actualHash = hash_file('sha256', $manifest->path());
        if ($actualHash === false || !hash_equals($manifest->hash(), $actualHash)) {
            throw new ExtensionLoadException('Native bridge hash does not match the pathmap.');
        }

        return NativeLibrary::load(
            $this->generatedPath($class, $ffiFile),
            $manifest->path(),
            $this->generatedPath($class, $this->aliasesFile())
        );
    }

    /** Lazily create and reuse a single {@see Allocator} for this runtime. */
    public function allocator(): Allocator
    {
        return $this->allocator ??= new Allocator();
    }

    /**
     * Run a callback with this runtime's project root published as the active
     * scope, returning whatever the callback returns.
     *
     * Nested loads triggered by generated entrypoints (which may build a Runtime
     * without an explicit root) inherit this root, and the previous scope is
     * always restored afterwards.
     *
     * @template T
     *
     * @param callable(): T $callback
     *
     * @return T
     */
    private function withProjectRootScope(callable $callback): mixed
    {
        // Generated entrypoints are regular PHP files, so root scoping is process-local.
        $previous = self::$activeProjectRoot;
        self::$activeProjectRoot = $this->projectRoot();

        try {
            return $callback();
        } finally {
            self::$activeProjectRoot = $previous;
        }
    }
}
