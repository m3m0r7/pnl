<?php

declare(strict_types=1);

namespace Pnlx;

/**
 * Reads and decodes a workspace JSON file after validating it against a schema.
 *
 * Used by {@see WorkspaceRepository} to load `pnl.json`, the pathmap, and each
 * `pnlx.json` manifest. Implementations are expected to reject files that fail
 * OpenAPI schema validation or do not decode to a JSON object.
 */
interface JsonReaderInterface
{
    /**
     * Validate the file against the named schema, then decode it to an array.
     *
     * @param string $path   Absolute path to the JSON file.
     * @param string $schema Schema identifier (e.g. `pnl`, `pnlx`, `pnlx-pathmap`).
     * @return array<string, mixed> The decoded JSON object.
     */
    public function read(string $path, string $schema): array;
}
