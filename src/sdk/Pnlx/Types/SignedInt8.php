<?php

declare(strict_types=1);

namespace Pnlx\Types;

class SignedInt8 extends AbstractInteger
{
    protected const string C_TYPE = 'int';
    protected const UNSIGNED = false;
}
