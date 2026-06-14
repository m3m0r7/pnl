<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

/**
 * Sanity-checks every bundled JSON schema under schemas/: that it parses and has
 * the expected OpenAPI shape. Strict OpenAPI validation is owned by the Rust
 * toolchain, which compiles these schemas (a malformed schema breaks `cargo test`),
 * so the SDK no longer depends on a PHP OpenAPI validator.
 */
final class SchemaFormatTest extends TestCase
{
    public function testSchemaFilesAreDiscovered(): void
    {
        self::assertNotEmpty(self::schemaPaths(), 'no schema files found under schemas/');
    }

    /**
     * @param string $path
     */
    #[DataProvider('schemaFileProvider')]
    public function testSchemaHasExpectedOpenApiShape(string $path): void
    {
        $raw = file_get_contents($path);
        self::assertIsString($raw, "{$path} could not be read");

        $document = json_decode($raw, true, flags: JSON_THROW_ON_ERROR);
        self::assertIsArray($document, "{$path} is not a JSON object");
        self::assertArrayHasKey('openapi', $document, "{$path} is missing the openapi version");

        self::assertArrayHasKey('components', $document, "{$path} is missing components");
        $components = $document['components'];
        self::assertIsArray($components, "{$path} components is not an object");

        self::assertArrayHasKey('schemas', $components, "{$path} is missing components.schemas");
        $schemas = $components['schemas'];
        self::assertIsArray($schemas, "{$path} components.schemas is not an object");

        self::assertArrayHasKey('Document', $schemas, "{$path} is missing components.schemas.Document");
    }

    /**
     * @return array<string, array{string}>
     */
    public static function schemaFileProvider(): array
    {
        $cases = [];
        foreach (self::schemaPaths() as $path) {
            // e.g. "pnlx-lock/2026-07-01"
            $label = basename(dirname(dirname($path))) . '/' . basename(dirname($path));
            $cases[$label] = [$path];
        }

        return $cases;
    }

    /**
     * @return list<string>
     */
    private static function schemaPaths(): array
    {
        $paths = glob(dirname(__DIR__) . '/schemas/*/*/schema.json');

        return $paths === false ? [] : $paths;
    }
}
