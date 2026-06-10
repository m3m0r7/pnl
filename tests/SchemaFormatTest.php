<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use cebe\openapi\Reader;
use cebe\openapi\spec\OpenApi;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

/**
 * Validates that every bundled JSON schema under schemas/ is a well-formed
 * OpenAPI document, using cebe/php-openapi as the OpenAPI format validator.
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
    public function testSchemaIsValidOpenApi(string $path): void
    {
        $document = Reader::readFromJsonFile($path, OpenApi::class, false);

        self::assertTrue(
            $document->validate(),
            sprintf(
                "%s is not a valid OpenAPI document:\n%s",
                $path,
                implode("\n", $document->getErrors())
            )
        );
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
