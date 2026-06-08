<?php

declare(strict_types=1);

namespace Pnlx;

use JsonException;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\FFIVerifier;
use Pnlx\Schema\OpenApiSchemaRepository;
use Pnlx\Schema\OpenApiSchemaValidator;

class Verifier
{
    public static function shouldEnabledFFI(): void
    {
        (new FFIVerifier())->shouldBeEnabled();
    }

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
            throw new ExtensionLoadException(sprintf(
                '%s does not match OpenAPI schema: %s',
                $path,
                implode('; ', array_slice($errors, 0, 5))
            ));
        }
    }
}
