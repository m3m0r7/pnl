<?php

declare(strict_types=1);

namespace Pnlx\FFI;

/**
 * How an exported data symbol is consumed by the C API.
 *
 * `Address` — the symbol is a value (e.g. a struct instance like oniguruma's
 * `OnigEncodingUTF8`) and the API wants a pointer to it (`&OnigEncodingUTF8`).
 * `Value` — the symbol is already a pointer (e.g. `OnigDefaultSyntax`) and the API
 * wants its value.
 */
enum SymbolMode
{
    case Address;
    case Value;
}
