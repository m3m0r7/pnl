<?php

declare(strict_types=1);

namespace Pnlx\FFI;

/**
 * The C scalar types an {@see Allocator} can allocate, mapped to the FFI type
 * declaration each one uses. The backing value IS the C type passed to FFI.
 *
 * Use it with {@see Allocator::make()}:
 *
 *     $int = $runtime->allocator()->make(AllocationType::Int);
 *     $big = $runtime->allocator()->make(AllocationType::Int64);
 */
enum AllocationType: string
{
    case Int = 'int';
    case Int8 = 'int8_t';
    case Int16 = 'int16_t';
    case Int32 = 'int32_t';
    case Int64 = 'int64_t';
    case UInt8 = 'uint8_t';
    case UInt16 = 'uint16_t';
    case UInt32 = 'uint32_t';
    case UInt64 = 'uint64_t';
    case Float = 'float';
    case Double = 'double';
    case Char = 'char';
    case Bool = 'bool';
    case VoidPointer = 'void *';

    /** The C type declaration FFI uses to allocate this value. */
    public function cType(): string
    {
        return $this->value;
    }
}
