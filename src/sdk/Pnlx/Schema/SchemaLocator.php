<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

/**
 * Maps a logical schema name and version to a bundled OpenAPI schema file.
 *
 * Schemas live under the repo's `schemas/<dir>/<version>/schema.json`. Used by
 * {@see OpenApiSchemaRepository} to find the schema matching a file's `schema_version`.
 */
class SchemaLocator
{
    /**
     * Allowed schema identifiers mapped to their `schemas/` subdirectory.
     *
     * @var array<string, string>
     */
    private const DIRECTORIES = [
        'pnl' => 'pnl',
        'pnlx' => 'pnlx',
        'pnlx-lock' => 'pnlx-lock',
        'pnlx-pathmap' => 'pnlx-pathmap',
        'repository-index' => 'repository-index',
    ];

    /**
     * Resolve the absolute path of a schema file for the given name and version.
     *
     * @throws ExtensionLoadException When the schema name is unknown or the file is missing.
     */
    public function locate(string $schema, string $version): string
    {
        if (!isset(self::DIRECTORIES[$schema])) {
            throw new ExtensionLoadException(sprintf('Unknown schema %s.', $schema));
        }

        // __DIR__ is src/sdk/Pnlx/Schema; four levels up reaches the repo root holding `schemas/`.
        $path = dirname(__DIR__, 4) . '/schemas/' . self::DIRECTORIES[$schema] . '/' . $version . '/schema.json';
        if (!is_file($path)) {
            throw new ExtensionLoadException(sprintf('Schema file %s does not exist.', $path));
        }

        return $path;
    }
}
