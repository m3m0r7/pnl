<?php

declare(strict_types=1);

namespace Pnlx\Version;

use Closure;
use InvalidArgumentException;

/**
 * A version-constraint expression, the PHP counterpart of the CLI's Rust
 * `version.rs`. Constraints combine semver comparators with `&` (and) and `|`
 * (or), where `&` binds tighter than `|`, and `()` groups sub-expressions:
 *
 *     1.2.3
 *     >=1.2.3 & <2.0.0
 *     >=1.2.3 & <2.0.0 | >=3.0.0
 *     (>=1.2.3 & <2.0.0) | >=3.0.0
 *
 * Parsing is a genuine two-stage pipeline: {@see tokenize()} is the lexer and
 * {@see parseOr()}/{@see parseAnd()}/{@see parseTerm()} form a recursive-descent
 * analyzer that builds a predicate tree. Whitespace is insignificant; two
 * comparators are only combined when an explicit `&`/`|` separates them.
 */
final class VersionConstraint
{
    /**
     * @param Closure(Semver):bool $predicate
     */
    private function __construct(private readonly Closure $predicate)
    {
    }

    public static function parse(string $input): self
    {
        $tokens = self::tokenize($input);
        if ($tokens === []) {
            throw new InvalidArgumentException("invalid version constraint: \"{$input}\" is empty");
        }

        $position = 0;
        $predicate = self::parseOr($tokens, $position);
        if ($position !== count($tokens)) {
            throw new InvalidArgumentException(
                "invalid version constraint \"{$input}\": expected '&', '|' or end of input",
            );
        }

        return new self($predicate);
    }

    public function matches(string $version): bool
    {
        return ($this->predicate)(Semver::parse($version));
    }

    /**
     * Convenience: parse `$constraint` and test `$version` in one call.
     */
    public static function satisfies(string $version, string $constraint): bool
    {
        return self::parse($constraint)->matches($version);
    }

    /**
     * The lexer: turn the raw expression into a flat token list.
     *
     * @return list<array{kind:string, op?:string, version?:string}>
     */
    private static function tokenize(string $input): array
    {
        $chars = str_split($input);
        $length = count($chars);
        $tokens = [];
        $index = 0;

        while ($index < $length) {
            $char = $chars[$index];
            if (ctype_space($char)) {
                $index++;
                continue;
            }

            $simple = match ($char) {
                '&' => 'and',
                '|' => 'or',
                '(' => 'open',
                ')' => 'close',
                default => null,
            };
            if ($simple !== null) {
                $tokens[] = ['kind' => $simple];
                $index++;
                continue;
            }

            if (str_contains('<>=^~', $char) || ctype_digit($char)) {
                $operatorStart = $index;
                while ($index < $length && str_contains('<>=^~', $chars[$index])) {
                    $index++;
                }
                $operator = implode('', array_slice($chars, $operatorStart, $index - $operatorStart));

                $versionStart = $index;
                while ($index < $length
                    && (ctype_alnum($chars[$index]) || $chars[$index] === '.'
                        || $chars[$index] === '-' || $chars[$index] === '+')) {
                    $index++;
                }
                $version = implode('', array_slice($chars, $versionStart, $index - $versionStart));
                if ($version === '') {
                    throw new InvalidArgumentException(
                        "invalid version constraint \"{$input}\": expected a version after \"{$operator}\"",
                    );
                }

                $tokens[] = ['kind' => 'comparator', 'op' => $operator, 'version' => $version];
                continue;
            }

            throw new InvalidArgumentException(
                "invalid version constraint \"{$input}\": unexpected character \"{$char}\"",
            );
        }

        return $tokens;
    }

    /**
     * @param list<array{kind:string, op?:string, version?:string}> $tokens
     * @return Closure(Semver):bool
     */
    private static function parseOr(array $tokens, int &$position): Closure
    {
        $items = [self::parseAnd($tokens, $position)];
        while (($tokens[$position]['kind'] ?? null) === 'or') {
            $position++;
            $items[] = self::parseAnd($tokens, $position);
        }

        return static fn (Semver $version): bool => self::any($items, $version);
    }

    /**
     * @param list<array{kind:string, op?:string, version?:string}> $tokens
     * @return Closure(Semver):bool
     */
    private static function parseAnd(array $tokens, int &$position): Closure
    {
        $items = [self::parseTerm($tokens, $position)];
        while (($tokens[$position]['kind'] ?? null) === 'and') {
            $position++;
            $items[] = self::parseTerm($tokens, $position);
        }

        return static fn (Semver $version): bool => self::all($items, $version);
    }

    /**
     * @param list<array{kind:string, op?:string, version?:string}> $tokens
     * @return Closure(Semver):bool
     */
    private static function parseTerm(array $tokens, int &$position): Closure
    {
        $token = $tokens[$position] ?? null;
        if ($token === null) {
            throw new InvalidArgumentException(
                "invalid version constraint: expected a version comparator or '('",
            );
        }

        if ($token['kind'] === 'open') {
            $position++;
            $inner = self::parseOr($tokens, $position);
            if (($tokens[$position]['kind'] ?? null) !== 'close') {
                throw new InvalidArgumentException("invalid version constraint: expected ')'");
            }
            $position++;

            return $inner;
        }

        if ($token['kind'] === 'comparator') {
            $position++;

            return self::comparator($token['op'] ?? '', $token['version'] ?? '');
        }

        throw new InvalidArgumentException(
            "invalid version constraint: expected a version comparator or '('",
        );
    }

    /**
     * @param list<Closure(Semver):bool> $items
     */
    private static function any(array $items, Semver $version): bool
    {
        foreach ($items as $item) {
            if ($item($version)) {
                return true;
            }
        }

        return false;
    }

    /**
     * @param list<Closure(Semver):bool> $items
     */
    private static function all(array $items, Semver $version): bool
    {
        foreach ($items as $item) {
            if (!$item($version)) {
                return false;
            }
        }

        return true;
    }

    /**
     * Build a single comparator predicate, expanding `^`/`~` into bounded ranges
     * the way Cargo's semver does.
     *
     * @return Closure(Semver):bool
     */
    private static function comparator(string $operator, string $versionText): Closure
    {
        $version = Semver::parse($versionText);

        return match ($operator) {
            '', '=', '==' => static fn (Semver $value): bool => $value->compareTo($version) === 0,
            '>' => static fn (Semver $value): bool => $value->compareTo($version) > 0,
            '>=' => static fn (Semver $value): bool => $value->compareTo($version) >= 0,
            '<' => static fn (Semver $value): bool => $value->compareTo($version) < 0,
            '<=' => static fn (Semver $value): bool => $value->compareTo($version) <= 0,
            '^' => self::boundedRange($version, self::caretUpperBound($version)),
            '~' => self::boundedRange(
                $version,
                Semver::parse(sprintf('%d.%d.0', $version->major, $version->minor + 1)),
            ),
            default => throw new InvalidArgumentException(
                "invalid version operator \"{$operator}\"",
            ),
        };
    }

    /**
     * @return Closure(Semver):bool
     */
    private static function boundedRange(Semver $lower, Semver $upper): Closure
    {
        return static fn (Semver $value): bool => $value->compareTo($lower) >= 0
            && $value->compareTo($upper) < 0;
    }

    private static function caretUpperBound(Semver $version): Semver
    {
        if ($version->major > 0) {
            return Semver::parse(sprintf('%d.0.0', $version->major + 1));
        }
        if ($version->minor > 0) {
            return Semver::parse(sprintf('0.%d.0', $version->minor + 1));
        }

        return Semver::parse(sprintf('0.0.%d', $version->patch + 1));
    }
}
