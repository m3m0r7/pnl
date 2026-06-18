<?php

declare(strict_types=1);

namespace Pnlx\Types;

class UnsignedInt32 extends AbstractInteger
{
    protected const string C_TYPE = 'unsigned int';
    protected const UNSIGNED = true;
}
