<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\Allocator;
use Pnlx\FFI\NativeLibrary;

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

    private ?Util $utilities = null;

    public function __construct(
        ?string $projectRoot = null,
        ?RuntimeConfigInterface $config = null,
        ?WorkspaceRepositoryInterface $repository = null,
        ?ExtensionRegistryInterface $registry = null,
    ) {
        Verifier::shouldEnabledFFI();

        $this->config = $config ?? new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $jsonReader = new JsonReader();
        $this->repository = $repository ?? new WorkspaceRepository($this->config, $jsonReader);
        $this->registry = $registry ?? new ExtensionRegistry($this->config, $this->repository);
    }

    public static function enableFunctions(?string $projectRoot = null): bool
    {
        $config = new RuntimeConfig($projectRoot ?? self::$activeProjectRoot);
        $manifest = (new WorkspaceRepository($config, new JsonReader()))->pnlManifest();
        $enables = $manifest['enables'] ?? [];

        return is_array($enables) && ($enables['use_functions'] ?? false) === true;
    }

    public function load(string $class): object
    {
        $this->withProjectRootScope(fn () => $this->registry->loadEntrypoint($class));

        if (!class_exists($class)) {
            throw new ExtensionLoadException(sprintf('Extension class %s was not loaded.', $class));
        }

        return new $class($this);
    }

    public function context(string $class): ContextInterface
    {
        $this->withProjectRootScope(fn () => $this->registry->loadEntrypoint($class));

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

    public function extensionRoot(string $class): string
    {
        return $this->registry->definition($class)->extensionRoot();
    }

    public function projectRoot(): string
    {
        return $this->config->projectRoot();
    }

    public function manifest(string $class): array
    {
        return $this->registry->definition($class)->manifest();
    }

    public function pathmap(): array
    {
        return $this->repository->pathmap();
    }

    public function generatedPath(string $class, string $file): string
    {
        return $this->extensionRoot($class) . '/' . $this->config->generatedDir() . '/' . $file;
    }

    public function aliasesFile(): string
    {
        return $this->config->aliasesFile();
    }

    public function native(string $class, string $ffiFile): NativeLibrary
    {
        $context = $this->context($class);
        if (!is_file($context->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }

        return NativeLibrary::load(
            $this->generatedPath($class, $ffiFile),
            $context->path(),
            $this->generatedPath($class, $this->aliasesFile())
        );
    }

    public function utilities(): Util
    {
        return $this->utilities ??= new Util();
    }

    public function allocator(): Allocator
    {
        return $this->allocator ??= new Allocator();
    }

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
