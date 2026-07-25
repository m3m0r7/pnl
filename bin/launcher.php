<?php

declare(strict_types=1);

use Pnlx\Cli\NativeBinaryLocator;

require_once dirname(__DIR__) . '/src/sdk/Pnlx/Cli/NativeBinaryLocator.php';

try {
    $binaryPath = NativeBinaryLocator::ensure(dirname(__DIR__), $pnlBinaryName);
} catch (Throwable $exception) {
    fwrite(STDERR, $exception->getMessage() . PHP_EOL);
    exit(1);
}

$arguments = [$binaryPath, ...array_slice($argv, 1)];
$process = @proc_open($arguments, [STDIN, STDOUT, STDERR], $pipes);
if (!is_resource($process)) {
    fwrite(STDERR, sprintf('pnl: failed to start %s%s', $pnlBinaryName, PHP_EOL));
    exit(1);
}

exit(proc_close($process));
