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

    public function testCallReinterpretsPointerFromAnotherFfiScope(): void
    {
        if (PHP_OS_FAMILY === 'Windows') {
            self::markTestSkipped('gettimeofday is not available on Windows.');
        }

        $cdef = <<<'CDEF'
            struct timeval {
                long tv_sec;
                long tv_usec;
            };
            int gettimeofday(struct timeval *tv, void *timezone);
            CDEF;
        $target = FFI::cdef($cdef);
        $foreign = FFI::cdef($cdef);
        $time = $foreign->new('struct timeval');
        self::assertInstanceOf(CData::class, $time);
        $empty = FFI::string(FFI::addr($time), FFI::sizeof($time));
        $library = new NativeLibrary($target, []);

        $result = $library->call('gettimeofday', [FFI::addr($time), null]);

        self::assertSame(0, $result);
        self::assertNotSame($empty, FFI::string(FFI::addr($time), FFI::sizeof($time)));
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
