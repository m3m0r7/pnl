<?php

declare(strict_types=1);

namespace Pnlx\Types;

class UnsignedLong extends AbstractInteger
{
    protected const string C_TYPE = 'unsigned long';
    protected const UNSIGNED = true;
}
