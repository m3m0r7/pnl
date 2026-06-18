<?php

declare(strict_types=1);

namespace Pnlx\Types;

class Int64T extends AbstractInteger
{
    protected const string C_TYPE = 'long long';
    protected const UNSIGNED = false;
}
