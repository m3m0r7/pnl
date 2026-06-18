<?php

declare(strict_types=1);

namespace Pnlx\Types;

class UnsignedInt8T extends AbstractInteger
{
    protected const string C_TYPE = 'unsigned char';
    protected const UNSIGNED = true;
}
