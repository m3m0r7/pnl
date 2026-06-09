<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Minimal recursive validator for the subset of OpenAPI/JSON Schema the SDK uses.
 *
 * It walks data against an {@see OpenApiSchemaDocument}, following `$ref`s and
 * checking types, `enum`, object required/properties/additionalProperties,
 * array `minItems`/`uniqueItems`/`items`, and string
 * `minLength`/`pattern`/`format`. Returns a flat list of human-readable error
 * messages (empty when valid) rather than throwing on validation failures.
 * Used by {@see \Pnlx\Verifier::shouldMatchSchema()}.
 */
class OpenApiSchemaValidator
{
    /**
     * Validate data against the document's root schema.
     *
     * @return list<string> Validation error messages; empty when the data is valid.
     */
    public function validate(mixed $data, OpenApiSchemaDocument $document): array
    {
        return $this->validateSchema($data, $document->rootSchema(), $document, '$');
    }

    /**
     * Recursively validate a value against a single schema node.
     *
     * @param array<string, mixed> $schema Schema node (may be a `$ref` to resolve).
     * @param string               $path   JSON-path-like location used in error messages.
     * @return list<string>
     * @throws ExtensionLoadException When a `$ref` value is not a string.
     */
    private function validateSchema(mixed $data, array $schema, OpenApiSchemaDocument $document, string $path): array
    {
        if (isset($schema['$ref'])) {
            if (!is_string($schema['$ref'])) {
                throw new ExtensionLoadException(sprintf('Invalid schema reference at %s.', $path));
            }

            return $this->validateSchema($data, $document->resolveRef($schema['$ref']), $document, $path);
        }

        // Null is only acceptable when the schema explicitly marks the node nullable.
        if ($data === null) {
            return ($schema['nullable'] ?? false) === true ? [] : [sprintf('%s must not be null.', $path)];
        }

        $errors = [];
        if (isset($schema['type']) && is_string($schema['type'])) {
            $errors = array_merge($errors, $this->validateType($data, $schema['type'], $path));
            // A type mismatch short-circuits; deeper checks would be meaningless.
            if ($errors !== []) {
                return $errors;
            }
        }

        if (isset($schema['enum']) && is_array($schema['enum']) && !in_array($data, $schema['enum'], true)) {
            $errors[] = sprintf('%s must be one of the allowed enum values.', $path);
        }

        return array_merge($errors, match ($schema['type'] ?? null) {
            'object' => $this->validateObject($data, $schema, $document, $path),
            'array' => $this->validateArray($data, $schema, $document, $path),
            'string' => $this->validateString($data, $schema, $path),
            default => [],
        });
    }

    /**
     * Check that a value matches the declared primitive/composite type.
     *
     * @return list<string>
     */
    private function validateType(mixed $data, string $type, string $path): array
    {
        $valid = match ($type) {
            'object' => is_object($data),
            'array' => is_array($data),
            'string' => is_string($data),
            'integer' => is_int($data),
            'number' => is_int($data) || is_float($data),
            'boolean' => is_bool($data),
            default => true,
        };

        return $valid ? [] : [sprintf('%s must be %s.', $path, $type)];
    }

    /**
     * Validate an object's required keys, declared properties, and additional properties.
     *
     * Also applies the `x-propertyNames` extension schema to each key name when present.
     *
     * @param array<string, mixed> $schema
     * @return list<string>
     */
    private function validateObject(mixed $data, array $schema, OpenApiSchemaDocument $document, string $path): array
    {
        // Type was already checked upstream; nothing to validate if it isn't an object.
        if (!is_object($data)) {
            return [];
        }

        $errors = [];
        $required = $schema['required'] ?? [];
        if (is_array($required)) {
            foreach ($required as $property) {
                if (is_string($property) && !property_exists($data, $property)) {
                    $errors[] = sprintf('%s.%s is required.', $path, $property);
                }
            }
        }

        $properties = isset($schema['properties']) && is_array($schema['properties']) ? $schema['properties'] : [];
        foreach ($properties as $property => $propertySchema) {
            if (!is_string($property) || !is_array($propertySchema) || !property_exists($data, $property)) {
                continue;
            }
            /** @var array<string, mixed> $propertySchema */
            $errors = array_merge(
                $errors,
                $this->validateSchema($data->{$property}, $propertySchema, $document, $path . '.' . $property)
            );
        }

        foreach (get_object_vars($data) as $property => $value) {
            $propertyPath = $path . '.' . $property;
            if (isset($schema['x-propertyNames']) && is_array($schema['x-propertyNames'])) {
                /** @var array<string, mixed> $propertyNamesSchema */
                $propertyNamesSchema = $schema['x-propertyNames'];
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($property, $propertyNamesSchema, $document, $propertyPath . '<key>')
                );
            }
            // Declared properties were validated above; only handle the rest here.
            if (array_key_exists($property, $properties)) {
                continue;
            }

            // Default is to permit extras; `false` forbids them, an array schema constrains them.
            $additional = $schema['additionalProperties'] ?? true;
            if ($additional === false) {
                $errors[] = sprintf('%s is not allowed.', $propertyPath);
            } elseif (is_array($additional)) {
                /** @var array<string, mixed> $additional */
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($value, $additional, $document, $propertyPath)
                );
            }
        }

        return $errors;
    }

    /**
     * Validate an array's `minItems`, `uniqueItems`, and per-item `items` schema.
     *
     * @param array<string, mixed> $schema
     * @return list<string>
     */
    private function validateArray(mixed $data, array $schema, OpenApiSchemaDocument $document, string $path): array
    {
        if (!is_array($data)) {
            return [];
        }

        $errors = [];
        if (isset($schema['minItems']) && is_int($schema['minItems']) && count($data) < $schema['minItems']) {
            $errors[] = sprintf('%s must contain at least %d items.', $path, $schema['minItems']);
        }
        // Compare serialized items so uniqueness works for scalars and nested structures alike.
        if (($schema['uniqueItems'] ?? false) === true && count($data) !== count(array_unique(array_map('serialize', $data)))) {
            $errors[] = sprintf('%s must contain unique items.', $path);
        }
        if (isset($schema['items']) && is_array($schema['items'])) {
            /** @var array<string, mixed> $itemsSchema */
            $itemsSchema = $schema['items'];
            foreach ($data as $index => $item) {
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($item, $itemsSchema, $document, sprintf('%s[%s]', $path, $index))
                );
            }
        }

        return $errors;
    }

    /**
     * Validate a string's `minLength`, `pattern`, and `format` (email/date-time).
     *
     * @param array<string, mixed> $schema
     * @return list<string>
     */
    private function validateString(mixed $data, array $schema, string $path): array
    {
        if (!is_string($data)) {
            return [];
        }

        $errors = [];
        if (isset($schema['minLength']) && is_int($schema['minLength']) && strlen($data) < $schema['minLength']) {
            $errors[] = sprintf('%s must be at least %d characters.', $path, $schema['minLength']);
        }
        // Use `~` as the delimiter, escaping any literal `~` in the pattern; @-suppress invalid patterns.
        if (isset($schema['pattern']) && is_string($schema['pattern']) && @preg_match('~' . str_replace('~', '\\~', $schema['pattern']) . '~', $data) !== 1) {
            $errors[] = sprintf('%s does not match pattern %s.', $path, $schema['pattern']);
        }
        if (($schema['format'] ?? null) === 'email' && !filter_var($data, FILTER_VALIDATE_EMAIL)) {
            $errors[] = sprintf('%s must be a valid email address.', $path);
        }
        if (($schema['format'] ?? null) === 'date-time' && strtotime($data) === false) {
            $errors[] = sprintf('%s must be a valid date-time.', $path);
        }

        return $errors;
    }
}
