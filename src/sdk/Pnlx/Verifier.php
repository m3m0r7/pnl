<?php

declare(strict_types=1);

namespace Pnlx;

use JsonException;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\FFIVerifier;
use Pnlx\Schema\OpenApiSchemaRepository;
use Pnlx\Schema\OpenApiSchemaValidator;

/**
 * Static facade for the SDK's pre-flight checks.
 *
 * Bundles the two assertions the runtime makes before trusting its environment or
 * input: that PHP FFI is usable ({@see FFIVerifier}) and that a workspace JSON file
 * conforms to its OpenAPI schema (via {@see OpenApiSchemaRepository} +
 * {@see OpenApiSchemaValidator}). Called by {@see Runtime} and {@see JsonReader}.
 */
class Verifier
{
    /**
     * Assert the FFI environment can load native bridges.
     *
     * @throws \Pnlx\Exception\FFIUnavailableException When FFI is missing or disabled.
     */
    public static function shouldEnabledFFI(): void
    {
        (new FFIVerifier())->shouldBeEnabled();
    }

    /**
     * Assert that a JSON file exists, parses, and matches the named OpenAPI schema.
     *
     * @param string $schema Schema identifier (e.g. `pnl`, `pnlx`).
     * @param string $path   Absolute path to the JSON file to validate.
     * @throws ExtensionLoadException When the file is missing, unreadable, malformed, or schema-invalid.
     */
    public static function shouldMatchSchema(string $schema, string $path): void
    {
        if (!is_file($path)) {
            throw new ExtensionLoadException(sprintf('Required file %s does not exist.', $path));
        }

        $json = file_get_contents($path);
        if ($json === false) {
            throw new ExtensionLoadException(sprintf('Failed to read %s.', $path));
        }

        try {
            $data = json_decode($json, false, flags: JSON_THROW_ON_ERROR);
        } catch (JsonException $exception) {
            throw new ExtensionLoadException(sprintf('Failed to parse %s: %s', $path, $exception->getMessage()), previous: $exception);
        }

        if (!is_object($data)) {
            throw new ExtensionLoadException(sprintf('%s must contain a JSON object.', $path));
        }

        $document = (new OpenApiSchemaRepository())->loadForData($schema, $data);
        $errors = (new OpenApiSchemaValidator())->validate($data, $document);
        if ($errors !== []) {
            // Surface only the first few errors to keep the message readable.
            throw new ExtensionLoadException(sprintf(
                '%s does not match OpenAPI schema: %s',
                $path,
                implode('; ', array_slice($errors, 0, 5))
            ));
        }
    }
}
