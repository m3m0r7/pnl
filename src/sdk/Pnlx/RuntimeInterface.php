<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\FFI\Allocator;
use Pnlx\FFI\NativeLibrary;

interface RuntimeInterface
{
    public function load(string $class): object;

    public function context(string $class): ContextInterface;

    public function extensionRoot(string $class): string;

    public function projectRoot(): string;

    /**
     * @return array<string, mixed>
     */
    public function manifest(string $class): array;

    /**
     * @return array<string, mixed>
     */
    public function pathmap(): array;

    public function generatedPath(string $class, string $file): string;

    public function aliasesFile(): string;

    public function native(string $class, string $ffiFile): NativeLibrary;

    public function utilities(): Util;

    public function allocator(): Allocator;
}
