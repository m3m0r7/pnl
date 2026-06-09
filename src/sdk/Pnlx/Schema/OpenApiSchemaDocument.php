<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Wraps a decoded OpenAPI document and exposes its schema definitions.
 *
 * Produced by {@see OpenApiSchemaRepository} and consumed by
 * {@see OpenApiSchemaValidator}: it provides the root `Document` schema to
 * validate against and resolves local `$ref` pointers into `components.schemas`.
 */
class OpenApiSchemaDocument
{
    /**
     * @param array<string, mixed> $document Decoded OpenAPI document.
     */
    public function __construct(
        private readonly array $document,
    ) {
    }

    /**
     * Return the top-level schema validation starts from (`components.schemas.Document`).
     *
     * @return array<string, mixed>
     * @throws ExtensionLoadException When the document lacks that schema.
     */
    public function rootSchema(): array
    {
        $components = $this->document['components'] ?? null;
        $schemas = is_array($components) ? ($components['schemas'] ?? null) : null;
        $schema = is_array($schemas) ? ($schemas['Document'] ?? null) : null;
        if (!is_array($schema)) {
            throw new ExtensionLoadException('OpenAPI schema is missing components.schemas.Document.');
        }

        /** @var array<string, mixed> $schema */
        return $schema;
    }

    /**
     * Resolve a local `#/components/schemas/<Name>` reference to its schema array.
     *
     * Only local component references are supported; external/remote refs are rejected.
     *
     * @return array<string, mixed>
     * @throws ExtensionLoadException When the ref is unsupported or the target schema is absent.
     */
    public function resolveRef(string $ref): array
    {
        $prefix = '#/components/schemas/';
        if (!str_starts_with($ref, $prefix)) {
            throw new ExtensionLoadException(sprintf('Unsupported schema reference %s.', $ref));
        }

        $name = substr($ref, strlen($prefix));
        $components = $this->document['components'] ?? null;
        $schemas = is_array($components) ? ($components['schemas'] ?? null) : null;
        $schema = is_array($schemas) ? ($schemas[$name] ?? null) : null;
        if (!is_array($schema)) {
            throw new ExtensionLoadException(sprintf('Schema reference %s does not exist.', $ref));
        }

        /** @var array<string, mixed> $schema */
        return $schema;
    }
}
