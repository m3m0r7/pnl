<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Schema\SchemaValidator;

/**
 * Default {@see JsonReaderInterface} that decodes a workspace JSON file.
 *
 * When a {@see SchemaValidator} is supplied (backed by the Rust support library
 * over FFI), the file is re-validated against its OpenAPI schema before decoding.
 * Schema validation is owned by Rust, so the runtime carries no OpenAPI
 * dependency; without the validator (library absent) it just decodes, trusting
 * the install-time validation.
 */
class JsonReader implements JsonReaderInterface
{
    public function __construct(private readonly ?SchemaValidator $validator = null)
    {
    }

    /**
     * @return array<string, mixed>
     * @throws ExtensionLoadException When the file cannot be read, fails schema validation, or does not decode to a JSON object.
     * @throws \JsonException When the file contains invalid JSON.
     */
    public function read(string $path, string $schema): array
    {
        $this->validator?->validate($schema, $path);

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
