<?php

declare(strict_types=1);

namespace Pnlx\Tests\Support;

class Filesystem
{
    public static function copyDirectory(string $from, string $to): void
    {
        mkdir($to, recursive: true);
        foreach (scandir($from) ?: [] as $entry) {
            if ($entry === '.' || $entry === '..') {
                continue;
            }

            $source = $from . '/' . $entry;
            $target = $to . '/' . $entry;
            if (is_dir($source)) {
                self::copyDirectory($source, $target);
            } else {
                copy($source, $target);
            }
        }
    }

    public static function removeDirectory(string $path): void
    {
        foreach (scandir($path) ?: [] as $entry) {
            if ($entry === '.' || $entry === '..') {
                continue;
            }

            $child = $path . '/' . $entry;
            if (is_dir($child)) {
                self::removeDirectory($child);
            } else {
                unlink($child);
            }
        }

        rmdir($path);
    }
}
