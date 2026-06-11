<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use InvalidArgumentException;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;
use Pnlx\Version\VersionConstraint;

final class VersionConstraintTest extends TestCase
{
    /**
     * @return iterable<string, array{string, string, bool}>
     */
    public static function satisfactionCases(): iterable
    {
        yield 'exact match' => ['1.2.3', '1.2.3', true];
        yield 'exact mismatch' => ['1.2.3', '1.2.4', false];
        yield 'explicit equals' => ['=1.2.3', '1.2.3', true];
        yield 'and range inside' => ['>=1.2.3 & <2.0.0', '1.5.0', true];
        yield 'and range outside' => ['>=1.2.3 & <2.0.0', '2.5.0', false];
        yield 'user example lower' => ['>3.0.0 & <4.0.0', '3.5.0', true];
        yield 'user example boundary' => ['>3.0.0 & <4.0.0', '3.0.0', false];
        yield 'user example upper' => ['>3.0.0 & <4.0.0', '4.0.0', false];
        yield 'or precedence low' => ['>=1.2.3 & <2.0.0 | >=3.0.0', '1.5.0', true];
        yield 'or precedence gap' => ['>=1.2.3 & <2.0.0 | >=3.0.0', '2.5.0', false];
        yield 'or precedence high' => ['>=1.2.3 & <2.0.0 | >=3.0.0', '3.1.0', true];
        yield 'grouped or' => ['(>=1.2.3 & <2.0.0) | >=3.0.0', '3.1.0', true];
        yield 'caret in range' => ['^1.2.3', '1.9.0', true];
        yield 'caret out of range' => ['^1.2.3', '2.0.0', false];
        yield 'caret zero major' => ['^0.2.3', '0.2.9', true];
        yield 'caret zero major bumped minor' => ['^0.2.3', '0.3.0', false];
        yield 'tilde in range' => ['~1.2.3', '1.2.9', true];
        yield 'tilde out of range' => ['~1.2.3', '1.3.0', false];
        yield 'whitespace tolerated' => ['  >=1.2.3  ', '1.2.3', true];
    }

    #[DataProvider('satisfactionCases')]
    public function testSatisfies(string $constraint, string $version, bool $expected): void
    {
        self::assertSame($expected, VersionConstraint::satisfies($version, $constraint));
    }

    /**
     * @return iterable<string, array{string}>
     */
    public static function invalidCases(): iterable
    {
        yield 'empty' => [''];
        yield 'whitespace implicit and' => ['>=1.2.0 <2.0.0'];
        yield 'trailing operator' => ['>=1.2.3 |'];
        yield 'leading operator' => ['& 1.2.3'];
        yield 'unbalanced paren' => ['(>=1.2.3 & <2.0.0'];
        yield 'dangling and' => ['1.2.3 & '];
        yield 'bad operator' => ['>>1.2.3'];
        yield 'unexpected character' => ['1.2.3 % 4'];
    }

    #[DataProvider('invalidCases')]
    public function testRejectsInvalidConstraints(string $constraint): void
    {
        $this->expectException(InvalidArgumentException::class);
        VersionConstraint::parse($constraint);
    }
}
