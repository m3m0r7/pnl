<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

class OpenApiSchemaDocument
{
    /**
     * @param array<string, mixed> $document
     */
    public function __construct(
        private readonly array $document,
    ) {
    }

    /**
     * @return array<string, mixed>
     */
    public function rootSchema(): array
    {
        $schema = $this->document['components']['schemas']['Document'] ?? null;
        if (!is_array($schema)) {
            throw new ExtensionLoadException('OpenAPI schema is missing components.schemas.Document.');
        }

        return $schema;
    }

    /**
     * @return array<string, mixed>
     */
    public function resolveRef(string $ref): array
    {
        $prefix = '#/components/schemas/';
        if (!str_starts_with($ref, $prefix)) {
            throw new ExtensionLoadException(sprintf('Unsupported schema reference %s.', $ref));
        }

        $name = substr($ref, strlen($prefix));
        $schema = $this->document['components']['schemas'][$name] ?? null;
        if (!is_array($schema)) {
            throw new ExtensionLoadException(sprintf('Schema reference %s does not exist.', $ref));
        }

        return $schema;
    }
}
