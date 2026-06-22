<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use FFI;
use FFI\CData;
use PHPUnit\Framework\TestCase;
use Pnlx\Util;

final class UtilTest extends TestCase
{
    protected function setUp(): void
    {
        if (!\extension_loaded('ffi')) {
            self::markTestSkipped('ext-ffi is required.');
        }
    }

    /**
     * Allocate a `wchar_t *` (4-byte/UTF-32, as on macOS/Linux) over the given code
     * points plus a NUL terminator, and decode it with {@see Util::wcStringOrNull}.
     * The owning buffer is held in a local for the duration of the call so the
     * pointer cast into it does not dangle.
     *
     * @param list<int> $codepoints
     */
    private function decode(array $codepoints): ?string
    {
        $codepoints[] = 0;
        // Allocate `int[]` (4-byte aligned) so the `int *` cast reads are aligned —
        // a `char[]` buffer would fault on a strict-alignment arch like arm64.
        $ffi = FFI::cdef('');
        $buffer = $ffi->new('int[' . \count($codepoints) . ']');
        if (!$buffer instanceof CData) {
            throw new \RuntimeException('FFI allocation failed.');
        }
        FFI::memcpy($buffer, pack('V*', ...$codepoints), \count($codepoints) * 4);

        return Util::wcStringOrNull($ffi->cast('int *', $buffer));
    }

    public function testDecodesUtf32IncludingAstralPlane(): void
    {
        // "Hi" + U+1F600 (😀), proving code points beyond the BMP round-trip.
        self::assertSame('Hi😀', $this->decode([0x48, 0x69, 0x1F600]));
    }

    public function testEmptyWideStringIsEmptyString(): void
    {
        self::assertSame('', $this->decode([]));
    }

    public function testNullIsNull(): void
    {
        self::assertNull(Util::wcStringOrNull(null));
    }
}
