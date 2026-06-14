<?php

declare(strict_types=1);

namespace Pnlx\Extension;

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\NativeLibrary;
use Pnlx\ManifestInterface;
use Pnlx\Runtime;
use Pnlx\RuntimeInterface;

/**
 * Base class shared by every generated extension entity. It owns the runtime
 * wiring and the magic methods/fields, so the generated subclass contains ONLY
 * the methods named after C functions — nothing here can be shadowed by a
 * generated method of the same name.
 */
abstract class AbstractExtension
{
    /** The package's generated FFI cdef file; each subclass overrides it. */
    protected const string FFI_FILE = '';

    protected NativeLibrary $native;

    protected RuntimeInterface $runtime;

    /** This extension's metadata; read it as `$ext->name` (see {@see __get()}). */
    public readonly ManifestInterface $manifest;

    /** Whether a raw PHP scalar may be passed as an argument (else it must be wrapped). */
    protected readonly bool $scalarParamsAllowed;

    public function __construct()
    {
        $this->runtime = new Runtime();
        $this->scalarParamsAllowed = Runtime::useScalarsInParams($this->runtime->projectRoot());
        $this->manifest = $this->runtime->loadManifest(static::class);

        if (!is_file($this->manifest->path())) {
            throw new ExtensionLoadException('an extension cannot be loaded');
        }

        $this->native = $this->runtime->native(static::class, static::FFI_FILE);
    }

    /**
     * @param list<mixed> $arguments
     */
    public function __call(string $name, array $arguments): mixed
    {
        return $this->native->call($name, $arguments);
    }

    /**
     * Read a manifest accessor as a field: `$ext->name` is `$ext->manifest->name()`.
     * Metadata stays reachable without a method call, so it never clashes with a
     * generated method named after a C function. `$ext->manifest` still works too.
     */
    public function __get(string $name): mixed
    {
        return match ($name) {
            'name' => $this->manifest->name(),
            'version' => $this->manifest->version(),
            'hash' => $this->manifest->hash(),
            'description' => $this->manifest->description(),
            'path' => $this->manifest->path(),
            default => throw new ExtensionLoadException(
                sprintf('Undefined property %s::$%s.', static::class, $name)
            ),
        };
    }
}
