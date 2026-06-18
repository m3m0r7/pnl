<?php

declare(strict_types=1);

namespace Pnlx\Types;

class SignedChar extends AbstractInteger
{
    protected const string C_TYPE = 'signed char';
    protected const UNSIGNED = false;
}
