<?php

declare(strict_types=1);

namespace Pnlx\FFI;

use FFI;
use FFI\CData;
use LogicException;

/**
 * 完全な cdef 本体を出力できない aggregate 用のアライン済み実体領域を所有し、
 * native 関数へ渡す型付きポインタを公開する。
 *
 * @internal
 */
final class OpaqueAllocation
{
    public function __construct(
        private readonly CData $pointer,
        private readonly CData $storage,
    ) {
    }

    public function pointer(): CData
    {
        if (FFI::sizeof($this->storage) < 1) {
            throw new LogicException('Opaque allocation lost its backing storage.');
        }

        return $this->pointer;
    }
}
