<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Default {@see JsonReaderInterface} that schema-validates then decodes a file.
 *
 * Delegates schema validation to {@see Verifier::shouldMatchSchema()} before
 * decoding so malformed or non-conforming workspace files are rejected up front.
 */
class JsonReader implements JsonReaderInterface
{
    /**
     * @return array<string, mixed>
     * @throws ExtensionLoadException When the file cannot be read or does not decode to a JSON object.
     * @throws \JsonException When the file contains invalid JSON.
     */
    public function read(string $path, string $schema): array
    {
        // Reject anything that fails OpenAPI schema validation before we trust its contents.
        Verifier::shouldMatchSchema($schema, $path);

        $json = file_get_contents($path);
        if ($json === false) {
            throw new ExtensionLoadException(sprintf('Failed to read %s.', $path));
        }

        $data = json_decode($json, true, flags: JSON_THROW_ON_ERROR);
        if (!is_array($data)) {
            throw new ExtensionLoadException(sprintf('%s did not contain a JSON object.', $path));
        }

        /** @var array<string, mixed> $data */
        return $data;
    }
}
