<?php

declare(strict_types=1);

namespace Pnlx\Version;

use InvalidArgumentException;

/**
 * A small semantic-version value object with comparison, used by
 * {@see VersionConstraint} to evaluate version expressions.
 *
 * Parsing is intentionally lenient about the number of numeric components so it
 * also accepts the partial versions real C libraries report (e.g. `1.5`,
 * `74.2`, or a bare `20190702`); missing components default to zero. Build
 * metadata (`+...`) is ignored for ordering, as the SemVer spec requires.
 */
final class Semver
{
    /**
     * @param list<string> $pre the dot-separated pre-release identifiers
     */
    private function __construct(
        public readonly int $major,
        public readonly int $minor,
        public readonly int $patch,
        public readonly array $pre,
    ) {
    }

    public static function parse(string $input): self
    {
        $value = trim($input);
        if ($value === '') {
            throw new InvalidArgumentException('empty version string');
        }

        // Strip build metadata, then split off any pre-release tail.
        $value = explode('+', $value, 2)[0];
        $preParts = explode('-', $value, 2);
        $core = $preParts[0];
        $pre = isset($preParts[1]) && $preParts[1] !== ''
            ? explode('.', $preParts[1])
            : [];

        $components = explode('.', $core);
        $numbers = [0, 0, 0];
        foreach ($components as $index => $component) {
            if ($index > 2) {
                break;
            }
            if (!ctype_digit($component)) {
                throw new InvalidArgumentException("invalid version component in {$input}");
            }
            $numbers[$index] = (int) $component;
        }

        return new self($numbers[0], $numbers[1], $numbers[2], $pre);
    }

    /**
     * Compare two versions, returning -1, 0, or 1 like the spaceship operator.
     */
    public function compareTo(self $other): int
    {
        foreach ([
            [$this->major, $other->major],
            [$this->minor, $other->minor],
            [$this->patch, $other->patch],
        ] as [$left, $right]) {
            if ($left !== $right) {
                return $left <=> $right;
            }
        }

        return self::comparePre($this->pre, $other->pre);
    }

    /**
     * A version with no pre-release outranks one that has it; otherwise the
     * identifiers are compared field-by-field (numeric fields numerically).
     *
     * @param list<string> $left
     * @param list<string> $right
     */
    private static function comparePre(array $left, array $right): int
    {
        if ($left === [] && $right === []) {
            return 0;
        }
        if ($left === []) {
            return 1;
        }
        if ($right === []) {
            return -1;
        }

        $count = min(count($left), count($right));
        for ($index = 0; $index < $count; $index++) {
            $result = self::comparePreField($left[$index], $right[$index]);
            if ($result !== 0) {
                return $result;
            }
        }

        return count($left) <=> count($right);
    }

    private static function comparePreField(string $left, string $right): int
    {
        $leftNumeric = ctype_digit($left);
        $rightNumeric = ctype_digit($right);

        if ($leftNumeric && $rightNumeric) {
            return (int) $left <=> (int) $right;
        }
        // Numeric identifiers always have lower precedence than alphanumeric.
        if ($leftNumeric !== $rightNumeric) {
            return $leftNumeric ? -1 : 1;
        }

        return strcmp($left, $right);
    }
}
