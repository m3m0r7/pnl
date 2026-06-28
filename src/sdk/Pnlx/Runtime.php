<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Attribute\NativeLibraryComponent;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Extension\AbstractExtension;

/**
 * Central entry point that wires together the SDK and loads extensions.
 *
 * On construction it verifies the FFI environment and assembles the default
 * collaborators: a {@see RuntimeConfig} (path/output-dir resolution), a
 * {@see WorkspaceRepository} (JSON manifests/lock/pathmap), and an
 * {@see ExtensionRegistry} (locating installed packages). It then loads generated
 * extension entrypoints and builds their `*Manifest` metadata. Each of the
 * collaborators can be injected for testing.
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
        // Fail fast if the FFI environment cannot support loading native libraries.
        Verifier::shouldEnabledFFI();

        // Prefer the caller's root, then the scope of an enclosing (generated) load, then cwd.
        $this->config = $config ?? new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $jsonReader = new JsonReader($this->schemaValidator());
        $this->repository = $repository ?? new WorkspaceRepository($this->config, $jsonReader);
        $this->registry = $registry ?? new ExtensionRegistry($this->config, $this->repository);
        // The self-contained type layer (Pnlx\Types\*) ships with the SDK and is
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
     * Reads `features.global_functions` from the manifest without constructing a full runtime.
     */
    public static function enableFunctions(?string $projectRoot = null): bool
    {
        return self::feature('global_functions', $projectRoot);
    }

    /**
     * Whether the workspace's `pnl.json` opts into exposing raw `\FFI\CData` in
     * generated signatures (the `cdata/<Class>.php` entity variant).
     *
     * Reads `features.cdata_arguments` from the manifest without constructing a full runtime.
     */
    public static function allowCData(?string $projectRoot = null): bool
    {
        return self::feature('cdata_arguments', $projectRoot);
    }

    /**
     * Whether `features.scalar_params` is on, i.e. generated methods
     * accept a raw PHP scalar argument (otherwise a raw scalar throws and the
     * caller must pass a `Pnlx\Types\*` wrapper).
     */
    public static function useScalarsInParams(?string $projectRoot = null): bool
    {
        return self::feature('scalar_params', $projectRoot, true);
    }

    /**
     * Whether `features.scalar_returns` is on, i.e. methods return PHP
     * native scalars (the `scalar/<Class>.php` entity variant) instead of wrappers.
     */
    public static function useScalarsInReturn(?string $projectRoot = null): bool
    {
        return self::feature('scalar_returns', $projectRoot);
    }

    /**
     * Whether `features.scalar_constants` is on, i.e. `const.php` uses PHP
     * native scalars for losslessly representable values (the `scalar/const.php`
     * variant) instead of `Pnlx\Types\*` wrappers.
     */
    public static function useScalarsInConst(?string $projectRoot = null): bool
    {
        return self::feature('scalar_constants', $projectRoot);
    }

    /**
     * Compose several generated extensions into ONE class that exposes all their
     * functions through a single shared FFI scope — so a CData produced by one
     * (e.g. an SDL2_image surface from `Sdlimage::IMG_Load`) flows straight into
     * another (`Libsdl::SDL_CreateTextureFromSurface`).
     *
     * It builds an anonymous class that `use`s each member's `<Class>LibraryComponent`
     * trait. Because the composed methods are REAL (not a `__call` proxy), by-reference
     * out-parameters round-trip: `$sdl = Runtime::compose([...]); $sdl->SDL_QueryTexture($t, $w, $h);`.
     * The shared scope is assembled lazily on first call by {@see AbstractExtension}
     * (its `class_uses()`-driven boot merges the components' cdefs and co-loads the
     * libraries).
     *
     * Equivalent to hand-writing `new class extends AbstractExtension { use ACompoent;
     * use BComponent; }`; `pnl compose --as <Class>` writes the same thing as a named
     * file for editor/static-analysis support.
     *
     * @param list<class-string<AbstractExtension>> $classes Generated entity classes (>= 2).
     *
     * @throws ExtensionLoadException When fewer than two classes are given or two
     *         members expose a same-named function (PHP would raise a trait conflict —
     *         use `pnl compose --prefix` to generate a renamed composite instead).
     */
    public static function compose(array $classes): object
    {
        if (count($classes) < 2) {
            throw new ExtensionLoadException('Runtime::compose() needs at least two extension classes.');
        }

        $traits = self::componentTraits($classes);
        self::assertNoComponentMethodCollisions($traits);

        $uses = '';
        foreach ($traits as $trait) {
            $uses .= '    use \\' . $trait . ";\n";
        }

        // The trait names come from reflecting the given classes (not user input). A
        // public constructor overrides AbstractExtension's private one so the
        // composite can be instantiated; the methods are static (callable through the
        // instance or the class), and boot wires the shared scope on first call.
        $composite = eval(
            "return new class extends \\Pnlx\\Extension\\AbstractExtension {\n"
            . "    public function __construct() {}\n"
            . $uses
            . '};'
        );
        if (!is_object($composite)) {
            throw new ExtensionLoadException('Runtime::compose() failed to build the composed class.');
        }

        return $composite;
    }

    /**
     * The `<Class>LibraryComponent` trait FQNs the given entity classes mix in
     * (a trait carrying `#[NativeLibraryComponent]` is a generated method group),
     * de-duplicated in first-seen order.
     *
     * @param list<class-string<AbstractExtension>> $classes
     * @return list<class-string>
     */
    private static function componentTraits(array $classes): array
    {
        $traits = [];
        foreach ($classes as $class) {
            foreach (class_uses($class) ?: [] as $trait) {
                if ((new \ReflectionClass($trait))->getAttributes(NativeLibraryComponent::class) !== []) {
                    $traits[$trait] = $trait;
                }
            }
        }

        return array_values($traits);
    }

    /**
     * Guard against composing components that expose a same-named method: PHP would
     * raise a fatal trait conflict, so fail early with an actionable message.
     *
     * @param list<class-string> $traits
     */
    private static function assertNoComponentMethodCollisions(array $traits): void
    {
        $seen = [];
        foreach ($traits as $trait) {
            foreach ((new \ReflectionClass($trait))->getMethods() as $method) {
                $key = strtolower($method->getName());
                if (isset($seen[$key]) && $seen[$key] !== $trait) {
                    throw new ExtensionLoadException(sprintf(
                        'Cannot compose: %s and %s both expose %s(). Generate a renamed composite with `pnl compose --prefix`.',
                        $seen[$key],
                        $trait,
                        $method->getName(),
                    ));
                }
                $seen[$key] = $trait;
            }
        }
    }

    /** Read a boolean `features.*` flag from `pnl.json` without a full runtime. */
    private static function feature(string $name, ?string $projectRoot, bool $default = false): bool
    {
        $config = new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $manifest = (new WorkspaceRepository($config, new JsonReader()))->pnlManifest();
        $features = $manifest['features'] ?? [];

        if (!is_array($features) || !array_key_exists($name, $features)) {
            return $default;
        }

        return $features[$name] === true;
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
