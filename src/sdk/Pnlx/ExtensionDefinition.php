<?php

declare(strict_types=1);

namespace Pnlx;

class ExtensionDefinition
{
    /**
     * @param array<string, mixed> $manifest
     */
    public function __construct(
        private readonly string $extensionRoot,
        private readonly array $manifest,
        private readonly string $class,
    ) {
    }

    public function extensionRoot(): string
    {
        return $this->extensionRoot;
    }

    /**
     * @return array<string, mixed>
     */
    public function manifest(): array
    {
        return $this->manifest;
    }

    public function class(): string
    {
        return $this->class;
    }
}
