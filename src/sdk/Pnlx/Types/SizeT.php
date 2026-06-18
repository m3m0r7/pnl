<?php

declare(strict_types=1);

namespace Pnlx\Types;

class SizeT extends AbstractInteger
{
    protected const string C_TYPE = 'size_t';
    protected const UNSIGNED = true;
}
