<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\ExtensionLoadException;

class JsonReader implements JsonReaderInterface
{
    public function read(string $path, string $schema): array
    {
        Verifier::shouldMatchSchema($schema, $path);

        $json = file_get_contents($path);
        if ($json === false) {
            throw new ExtensionLoadException(sprintf('Failed to read %s.', $path));
        }

        return json_decode($json, true, flags: JSON_THROW_ON_ERROR);
    }
}
