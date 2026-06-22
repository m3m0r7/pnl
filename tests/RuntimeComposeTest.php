<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\Attribute\NativeLibraryComponent;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\Extension\AbstractExtension;
use Pnlx\Runtime;

/** A stand-in generated method group (a Component trait). */
#[NativeLibraryComponent(ComposeAlpha::class)]
trait ComposeAlphaComponent
{
    public static function alpha_only(int $n): int
    {
        return $n;
    }

    public static function shared_fn(): string
    {
        return 'alpha';
    }
}

class ComposeAlpha extends AbstractExtension
{
    use ComposeAlphaComponent;
}

#[NativeLibraryComponent(ComposeBeta::class)]
trait ComposeBetaComponent
{
    public static function beta_only(int $n): int
    {
        return $n;
    }
}

class ComposeBeta extends AbstractExtension
{
    use ComposeBetaComponent;
}

/** Collides with Alpha on `shared_fn` to exercise the collision guard. */
#[NativeLibraryComponent(ComposeGamma::class)]
trait ComposeGammaComponent
{
    public static function shared_fn(): string
    {
        return 'gamma';
    }
}

class ComposeGamma extends AbstractExtension
{
    use ComposeGammaComponent;
}

class RuntimeComposeTest extends TestCase
{
    public function testComposeRequiresAtLeastTwoExtensions(): void
    {
        $this->expectException(ExtensionLoadException::class);
        $this->expectExceptionMessage('at least two');

        Runtime::compose([]);
    }

    public function testComposeBuildsAClassMixingEachComponent(): void
    {
        $composite = Runtime::compose([ComposeAlpha::class, ComposeBeta::class]);

        // The composite mixes in every member's Component trait, so it exposes all
        // their (real) methods — by-reference out-params therefore round-trip.
        $uses = class_uses($composite) ?: [];
        self::assertContains(ComposeAlphaComponent::class, $uses);
        self::assertContains(ComposeBetaComponent::class, $uses);
        self::assertTrue(method_exists($composite, 'alpha_only'));
        self::assertTrue(method_exists($composite, 'beta_only'));
    }

    public function testComposeRejectsCollidingMethodNames(): void
    {
        $this->expectException(ExtensionLoadException::class);
        $this->expectExceptionMessage('shared_fn');

        Runtime::compose([ComposeAlpha::class, ComposeGamma::class]);
    }
}
