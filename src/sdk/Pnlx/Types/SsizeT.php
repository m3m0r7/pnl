<?php

declare(strict_types=1);

namespace Pnlx\Types;

class SsizeT extends AbstractInteger
{
    protected const string C_TYPE = 'ssize_t';
    protected const UNSIGNED = false;
}
