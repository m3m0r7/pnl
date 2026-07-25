<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use FFI;
use FFI\CData;
use PHPUnit\Framework\TestCase;
use Pnlx\Exception\ExtensionLoadException;
use Pnlx\FFI\NativeLibrary;

class NativeLibraryTest extends TestCase
{
    protected function setUp(): void
    {
        if (!class_exists(FFI::class) || ini_get('ffi.enable') === '0') {
            self::markTestSkipped('PHP FFI is not enabled.');
        }
    }

    public function testOpaqueAllocationReturnsAlignedOwnedStorage(): void
    {
        $ffi = FFI::cdef(
            'struct ex_opaque;',
        );
        $library = new NativeLibrary($ffi, []);

        $allocation = $library->allocateOpaque('struct ex_opaque', 32, 16, 2);
        $pointer = $allocation->pointer();
        $addressValue = $ffi->cast(
            FFI::sizeof($pointer) === 8 ? 'unsigned long long' : 'unsigned int',
            $pointer,
        );
        self::assertInstanceOf(CData::class, $addressValue);
        $address = self::cdataInteger($addressValue);

        self::assertSame(0, $address % 16);
        self::assertSame('struct ex_opaque*', FFI::typeof($pointer)->getName());
    }

    public function testOpaqueAllocationRejectsInvalidAlignment(): void
    {
        $ffi = FFI::cdef(
            'struct ex_opaque;',
        );
        $library = new NativeLibrary($ffi, []);

        $this->expectException(ExtensionLoadException::class);
        $library->allocateOpaque('struct ex_opaque', 4, 3);
    }

    private static function cdataInteger(CData $value): int
    {
        $width = FFI::sizeof($value);
        $decoded = unpack(
            $width === 8 ? 'Qvalue' : 'Lvalue',
            FFI::string(FFI::addr($value), $width),
        );
        $integer = $decoded['value'] ?? null;
        self::assertIsInt($integer);

        return $integer;
    }
}
