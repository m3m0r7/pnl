<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\ComposedScope;
use Pnlx\Exception\NativeFunctionCallException;

/** A stand-in "member" whose static methods the scope should proxy to. */
final class ComposedScopeAlpha
{
    public static function alphaOnly(int $n): string
    {
        return 'alpha:' . $n;
    }

    public static function shared(): string
    {
        return 'alpha-shared';
    }
}

/** A second member; `shared()` overlaps with Alpha to exercise first-wins. */
final class ComposedScopeBeta
{
    public static function betaOnly(int $n): string
    {
        return 'beta:' . $n;
    }

    public static function shared(): string
    {
        return 'beta-shared';
    }
}

class ComposedScopeTest extends TestCase
{
    private function scope(): ComposedScope
    {
        return new ComposedScope(
            [ComposedScopeAlpha::class, ComposedScopeBeta::class],
            [
                'alphaonly' => ComposedScopeAlpha::class,
                'shared' => ComposedScopeAlpha::class, // first member wins
                'betaonly' => ComposedScopeBeta::class,
            ],
        );
    }

    public function testProxiesEachCallToItsOwningMember(): void
    {
        $scope = $this->scope();

        self::assertSame('alpha:7', $scope->__call('alphaOnly', [7]));
        self::assertSame('beta:9', $scope->__call('betaOnly', [9]));
    }

    public function testRoutingIsCaseInsensitiveAndFirstMemberWinsOnOverlap(): void
    {
        $scope = $this->scope();

        // Looked up case-insensitively (C names vs camelCase aliases).
        self::assertSame('alpha:1', $scope->__call('ALPHAONLY', [1]));
        // `shared` exists on both members; the first one registered wins.
        self::assertSame('alpha-shared', $scope->__call('shared', []));
    }

    public function testUnknownFunctionThrows(): void
    {
        $this->expectException(NativeFunctionCallException::class);
        $this->expectExceptionMessage('nope');

        $this->scope()->__call('nope', []);
    }

    public function testExtensionsAreReportedInOrder(): void
    {
        self::assertSame(
            [ComposedScopeAlpha::class, ComposedScopeBeta::class],
            $this->scope()->extensions(),
        );
    }
}
