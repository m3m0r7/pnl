<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use FFI;
use FFI\CData;
use PHPUnit\Framework\TestCase;
use Pnlx\FFI\ArgumentMarshaller;
use Pnlx\Types\ValueInterface;

final class ArgumentMarshallerTest extends TestCase
{
    protected function setUp(): void
    {
        if (!class_exists(FFI::class) || ini_get('ffi.enable') === '0') {
            self::markTestSkipped('PHP FFI is not enabled.');
        }
    }

    public function testStructValueDereferencesGeneratedWrapperArray(): void
    {
        $ffi = FFI::cdef('typedef struct { unsigned char red; } ex_color;');
        $array = $ffi->new('ex_color[1]');
        self::assertInstanceOf(CData::class, $array);
        FFI::memcpy($array, "\x2a", 1);
        $wrapper = new class ($array) implements ValueInterface {
            public function __construct(private readonly CData $value)
            {
            }

            public function toValue(): CData
            {
                return $this->value;
            }
        };

        $value = ArgumentMarshaller::structValue($wrapper);

        self::assertInstanceOf(CData::class, $value);
        self::assertSame(\FFI\CType::TYPE_STRUCT, FFI::typeof($value)->getKind());
        self::assertSame("\x2a", FFI::string(FFI::addr($value), 1));
    }

    public function testStructValueDereferencesPointer(): void
    {
        $ffi = FFI::cdef('typedef struct { unsigned char red; } ex_color;');
        $value = $ffi->new('ex_color');
        self::assertInstanceOf(CData::class, $value);
        FFI::memcpy(FFI::addr($value), "\x54", 1);

        $marshalled = ArgumentMarshaller::structValue(FFI::addr($value));

        self::assertInstanceOf(CData::class, $marshalled);
        self::assertSame(\FFI\CType::TYPE_STRUCT, FFI::typeof($marshalled)->getKind());
        self::assertSame("\x54", FFI::string(FFI::addr($marshalled), 1));
    }

    public function testStructValueKeepsByValueStruct(): void
    {
        $ffi = FFI::cdef('typedef struct { unsigned char red; } ex_color;');
        $value = $ffi->new('ex_color');
        self::assertInstanceOf(CData::class, $value);

        self::assertSame($value, ArgumentMarshaller::structValue($value));
    }
}
