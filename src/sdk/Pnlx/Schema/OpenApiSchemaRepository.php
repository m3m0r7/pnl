<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use cebe\openapi\Reader;
use cebe\openapi\spec\OpenApi;
use JsonException;
use Pnlx\Exception\ExtensionLoadException;

/**
 * Loads the versioned OpenAPI schema that matches a piece of workspace data.
 *
 * It reads `schema_version` off the decoded JSON, locates the matching schema file
 * via {@see SchemaLocator}, validates it with the cebe OpenAPI reader, then returns
 * a decoded {@see OpenApiSchemaDocument} for {@see OpenApiSchemaValidator} to use.
 */
class OpenApiSchemaRepository
{
    public function __construct(
        private readonly SchemaLocator $locator = new SchemaLocator(),
    ) {
    }

    /**
     * Locate, validate, and decode the schema document for the given data's version.
     *
     * @param string $schema Schema identifier (e.g. `pnl`, `pnlx`).
     * @param object $data    Decoded JSON whose `schema_version` selects the schema file.
     * @throws ExtensionLoadException When the version is missing, or the schema is invalid/unreadable/malformed.
     */
    public function loadForData(string $schema, object $data): OpenApiSchemaDocument
    {
        $version = $data->schema_version ?? null;
        if (!is_string($version) || $version === '') {
            throw new ExtensionLoadException('schema_version must be a non-empty string.');
        }

        $schemaPath = $this->locator->locate($schema, $version);
        // Suppress E_DEPRECATED emitted by the cebe OpenAPI reader on modern PHP.
        set_error_handler(static function (int $severity): bool {
            return $severity === E_DEPRECATED;
        }, E_DEPRECATED);
        try {
            $openApi = Reader::readFromJsonFile($schemaPath, OpenApi::class, false);
        } finally {
            restore_error_handler();
        }

        if (!$openApi->validate()) {
            throw new ExtensionLoadException(sprintf(
                'OpenAPI schema %s is invalid: %s',
                $schemaPath,
                implode('; ', $openApi->getErrors())
            ));
        }

        $json = file_get_contents($schemaPath);
        if ($json === false) {
            throw new ExtensionLoadException(sprintf('Failed to read schema %s.', $schemaPath));
        }

        try {
            $document = json_decode($json, true, flags: JSON_THROW_ON_ERROR);
        } catch (JsonException $exception) {
            throw new ExtensionLoadException(sprintf('Failed to parse schema %s: %s', $schemaPath, $exception->getMessage()), previous: $exception);
        }

        if (!is_array($document)) {
            throw new ExtensionLoadException(sprintf('Schema %s must contain a JSON object.', $schemaPath));
        }

        /** @var array<string, mixed> $document */
        return new OpenApiSchemaDocument($document);
    }
}
