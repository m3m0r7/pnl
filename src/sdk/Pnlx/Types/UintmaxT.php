<?php

declare(strict_types=1);

namespace Pnlx\Types;

class UintmaxT extends AbstractInteger
{
    protected const string C_TYPE = 'unsigned long long';
    protected const UNSIGNED = true;
}
