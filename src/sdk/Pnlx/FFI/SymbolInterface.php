<?php

declare(strict_types=1);

namespace Pnlx\FFI;

/**
 * A generated marker for one exported data symbol (a C global variable the typed
 * function bindings can't express, e.g. oniguruma's `OnigEncodingUTF8`).
 *
 * Each symbol is generated as a flat marker class (`\Pnlx\Liboniguruma\OnigEncodingUTF8`).
 * A caller passes the class-string (`...::class`) straight to any function that
 * takes the symbol; it is a cheap compile-time string, and the actual FFI value is
 * resolved lazily — only when {@see ArgumentMarshaller::unwrap()} sees the marker.
 */
interface SymbolInterface
{
    /**
     * The generated extension entity that owns this symbol.
     *
     * @return class-string<\Pnlx\Extension\AbstractExtension>
     */
    public static function extension(): string;

    /** The exported C symbol name. */
    public static function name(): string;

    /** Whether the API wants the symbol's address or its value. */
    public static function mode(): SymbolMode;
}
