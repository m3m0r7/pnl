<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

class OpenApiSchemaValidator
{
    /**
     * @return list<string>
     */
    public function validate(mixed $data, OpenApiSchemaDocument $document): array
    {
        return $this->validateSchema($data, $document->rootSchema(), $document, '$');
    }

    /**
     * @param array<string, mixed> $schema
     * @return list<string>
     */
    private function validateSchema(mixed $data, array $schema, OpenApiSchemaDocument $document, string $path): array
    {
        if (isset($schema['$ref'])) {
            if (!is_string($schema['$ref'])) {
                throw new ExtensionLoadException(sprintf('Invalid schema reference at %s.', $path));
            }

            return $this->validateSchema($data, $document->resolveRef($schema['$ref']), $document, $path);
        }

        if ($data === null) {
            return ($schema['nullable'] ?? false) === true ? [] : [sprintf('%s must not be null.', $path)];
        }

        $errors = [];
        if (isset($schema['type']) && is_string($schema['type'])) {
            $errors = array_merge($errors, $this->validateType($data, $schema['type'], $path));
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
     * @param array<string, mixed> $schema
     * @return list<string>
     */
    private function validateObject(mixed $data, array $schema, OpenApiSchemaDocument $document, string $path): array
    {
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
            $errors = array_merge(
                $errors,
                $this->validateSchema($data->{$property}, $propertySchema, $document, $path . '.' . $property)
            );
        }

        foreach (get_object_vars($data) as $property => $value) {
            $propertyPath = $path . '.' . $property;
            if (isset($schema['x-propertyNames']) && is_array($schema['x-propertyNames'])) {
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($property, $schema['x-propertyNames'], $document, $propertyPath . '<key>')
                );
            }
            if (array_key_exists($property, $properties)) {
                continue;
            }

            $additional = $schema['additionalProperties'] ?? true;
            if ($additional === false) {
                $errors[] = sprintf('%s is not allowed.', $propertyPath);
            } elseif (is_array($additional)) {
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($value, $additional, $document, $propertyPath)
                );
            }
        }

        return $errors;
    }

    /**
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
        if (($schema['uniqueItems'] ?? false) === true && count($data) !== count(array_unique(array_map('serialize', $data)))) {
            $errors[] = sprintf('%s must contain unique items.', $path);
        }
        if (isset($schema['items']) && is_array($schema['items'])) {
            foreach ($data as $index => $item) {
                $errors = array_merge(
                    $errors,
                    $this->validateSchema($item, $schema['items'], $document, sprintf('%s[%s]', $path, $index))
                );
            }
        }

        return $errors;
    }

    /**
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
