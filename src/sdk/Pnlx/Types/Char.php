<?php

declare(strict_types=1);

namespace Pnlx\Types;

class Char extends AbstractInteger
{
    protected const string C_TYPE = 'signed char';
    protected const UNSIGNED = false;
}
