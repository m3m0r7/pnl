<?php

declare(strict_types=1);

namespace Pnlx;

interface JsonReaderInterface
{
    /**
     * @return array<string, mixed>
     */
    public function read(string $path, string $schema): array;
}
