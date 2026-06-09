<?php

declare(strict_types=1);

namespace Pnlx\Tests\Support;

use PHPUnit\Framework\Assert;

class CommandRunner
{
    public function __construct(
        private readonly string $cwd,
    ) {
    }

    /**
     * @param list<string> $command
     * @param array<string, string>|null $env
     */
    public function run(array $command, ?array $env = null): void
    {
        /** @var array<string, string>|null $envVars */
        $envVars = $env === null ? null : array_merge($_ENV, $env);

        $process = proc_open(
            $command,
            [
                1 => ['pipe', 'w'],
                2 => ['pipe', 'w'],
            ],
            $pipes,
            $this->cwd,
            $envVars
        );

        if (!is_resource($process)) {
            Assert::fail('Failed to start process: ' . implode(' ', $command));
        }

        $stdout = stream_get_contents($pipes[1]);
        $stderr = stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        $status = proc_close($process);

        Assert::assertSame(0, $status, trim((string) $stdout . "\n" . (string) $stderr));
    }
}
