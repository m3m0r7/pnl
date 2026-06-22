<?php

declare(strict_types=1);

namespace Pnlx;

use Pnlx\Exception\NativeFunctionCallException;

/**
 * Handle returned by {@see Runtime::compose()} for a set of extensions sharing one
 * FFI scope. Proxies native calls to whichever member extension declares them, so
 * you can write `Runtime::compose([A::class, B::class])->some_c_function(...)`.
 *
 * The member entity classes also keep working as plain static calls
 * (`A::some_c_function(...)`) — they now dispatch through the same shared scope, so
 * a CData one returns can be passed straight into another.
 */
final class ComposedScope
{
    /**
     * @param list<class-string> $extensions   The composed member entity classes.
     * @param array<string, class-string> $owners Lower-cased function name => the member
     *                                            class that exposes it (first member wins).
     */
    public function __construct(
        private readonly array $extensions,
        private readonly array $owners,
    ) {
    }

    /**
     * The composed member entity classes, in composition order.
     *
     * @return list<class-string>
     */
    public function extensions(): array
    {
        return $this->extensions;
    }

    /**
     * Route a native call to the member extension that exposes it.
     *
     * @param list<mixed> $arguments
     */
    public function __call(string $name, array $arguments): mixed
    {
        $owner = $this->owners[strtolower($name)] ?? null;
        if ($owner === null) {
            throw new NativeFunctionCallException(
                sprintf('No composed extension exposes a function named "%s".', $name),
            );
        }

        return $owner::$name(...$arguments);
    }
}
