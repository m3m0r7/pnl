<?php

declare(strict_types=1);

namespace Pnlx\Schema;

use Pnlx\Exception\ExtensionLoadException;

class SchemaLocator
{
    /** @var array<string, string> */
    private const DIRECTORIES = [
        'pnl' => 'pnl',
        'pnlx' => 'pnlx',
        'pnlx-lock' => 'pnlx-lock',
        'pnlx-pathmap' => 'pnlx-pathmap',
        'repository-index' => 'repository-index',
    ];

    public function locate(string $schema, string $version): string
    {
        if (!isset(self::DIRECTORIES[$schema])) {
            throw new ExtensionLoadException(sprintf('Unknown schema %s.', $schema));
        }

        $path = dirname(__DIR__, 4) . '/schemas/' . self::DIRECTORIES[$schema] . '/' . $version . '/schema.json';
        if (!is_file($path)) {
            throw new ExtensionLoadException(sprintf('Schema file %s does not exist.', $path));
        }

        return $path;
    }
}
