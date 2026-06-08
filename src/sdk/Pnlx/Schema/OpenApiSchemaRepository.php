<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use cebe\openapi\Reader;
use cebe\openapi\spec\OpenApi;
use JsonException;
use Pnlx\Exception\ExtensionLoadException;

class OpenApiSchemaRepository
{
    public function __construct(
        private readonly SchemaLocator $locator = new SchemaLocator(),
    ) {
    }

    public function loadForData(string $schema, object $data): OpenApiSchemaDocument
    {
        $version = $data->schema_version ?? null;
        if (!is_string($version) || $version === '') {
            throw new ExtensionLoadException('schema_version must be a non-empty string.');
        }

        $schemaPath = $this->locator->locate($schema, $version);
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

        return new OpenApiSchemaDocument($document);
    }
}
