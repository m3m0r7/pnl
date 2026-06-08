<?php

declare(strict_types=1);

namespace Pnlx;

interface ExtensionRegistryInterface
{
    public function definition(string $class): ExtensionDefinition;

    public function loadEntrypoint(string $class): void;
}
