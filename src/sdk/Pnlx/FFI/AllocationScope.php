<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI\CData;
use Pnlx\Exception\PHPNativeLibraryException;

/**
 * A lifetime for a batch of C allocations.
 *
 * Every value allocated through the scope is retained until {@see release()} (or
 * the scope is garbage-collected), which guarantees the backing memory outlives
 * any native call that received a pointer into it — the common, easy-to-hit
 * use-after-free where PHP frees an allocation while C still holds its address.
 * Using a scope after it is released throws rather than handing back a value whose
 * lifetime is no longer managed.
 */
final class AllocationScope
{
    /**
     * Strong references to allocations, kept solely to pin their lifetime to the
     * scope; never read back.
     *
     * @var list<CData>
     */
    private array $retained = [];

    private bool $released = false;

    public function __construct(private readonly Allocator $allocator)
    {
    }

    /** Allocate a zero-initialized value of a C type, retained by this scope. */
    public function new(string $type): CData
    {
        $this->ensureLive();
        $value = $this->allocator->new($type);
        $this->retained[] = $value;

        return $value;
    }

    /** Allocate a NUL-terminated `char` buffer, retained by this scope. */
    public function cString(string $value): CData
    {
        $this->ensureLive();
        $buffer = $this->allocator->cString($value);
        $this->retained[] = $buffer;

        return $buffer;
    }

    /** How many allocations the scope is currently pinning. */
    public function count(): int
    {
        return \count($this->retained);
    }

    /**
     * Drop the scope's references to its allocations (so the GC may reclaim them)
     * and forbid further use. Call this once the native side no longer needs the
     * memory. Idempotent.
     */
    public function release(): void
    {
        $this->retained = [];
        $this->released = true;
    }

    public function __destruct()
    {
        $this->release();
    }

    private function ensureLive(): void
    {
        if ($this->released) {
            throw new PHPNativeLibraryException(
                'This AllocationScope has been released; allocate from a new scope.',
            );
        }
    }
}
