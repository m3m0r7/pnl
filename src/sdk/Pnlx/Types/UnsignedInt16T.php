<?php

declare(strict_types=1);

namespace Pnlx\Types;

class UnsignedInt16T extends AbstractInteger
{
    protected const string C_TYPE = 'unsigned short';
    protected const UNSIGNED = true;
}
