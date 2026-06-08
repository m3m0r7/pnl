<?php

declare(strict_types=1);

/*
 * This file is generated. Manual edits may be overwritten.
 * To customize behavior, add an overriding class under src/ and implement your methods there.
 *
 * Generated at: {{GENERATED_AT}}
 * Generated on: {{GENERATED_HOST}}
 * Generator OS: {{GENERATED_OS}}
 * PHP version: {{GENERATED_PHP_VERSION}}
 */

namespace {{NAMESPACE}};

use Pnlx\ContextInterface;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\RuntimeInterface;

class {{CLASS}}Context implements ContextInterface
{
    public function __construct(
        private readonly RuntimeInterface $runtime,
    ) {
    }

    public function name(): string
    {
        return $this->manifest()['name'];
    }

    public function version(): string
    {
        return $this->manifest()['version'];
    }

    public function hash(): string
    {
        return $this->bridge()['sha256'];
    }

    public function description(): string
    {
        return $this->manifest()['description'];
    }

    public function path(): string
    {
        return $this->absolutePath($this->bridge()['library']);
    }

    /**
     * @return array<string, mixed>
     */
    protected function manifest(): array
    {
        return $this->runtime->manifest({{CLASS}}::class);
    }

    /**
     * @return array{source: string, library: string, sha256: string}
     */
    protected function bridge(): array
    {
        $bridge = $this->runtime->pathmap()['bridges']['{{LIBRARY_KEY}}'] ?? null;
        if (!is_array($bridge)) {
            throw new ExtensionLoadException('Bridge {{LIBRARY_KEY}} is not installed.');
        }

        foreach (['source', 'library', 'sha256'] as $key) {
            if (!isset($bridge[$key]) || !is_string($bridge[$key]) || $bridge[$key] === '') {
                throw new ExtensionLoadException(sprintf('Bridge {{LIBRARY_KEY}} is missing %s.', $key));
            }
        }

        return $bridge;
    }

    protected function absolutePath(string $path): string
    {
        if (str_starts_with($path, '/')) {
            return $path;
        }

        return $this->runtime->projectRoot() . '/' . $path;
    }
}
