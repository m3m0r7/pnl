<?php

declare(strict_types=1);

/*
 * This file is generated. Manual edits may be overwritten.
 * To customize behavior, add an overriding class under src/ and implement your methods there.
 *
 * Generated at: {{GENERATED_AT}}
 * Generated on: {{GENERATED_HOST}}
 * Generator OS: {{GENERATED_OS}}
 * PHP version: {{GENERATED_PHP_VERSION}}
 */

require_once __DIR__ . '/{{CLASS}}Context.php';
require_once __DIR__ . '/{{CLASS}}.php';

$runtimeVarName = '{{RUNTIME_VAR}}';
$runtime = new \Pnlx\Runtime();

if (is_file(__DIR__ . '/preload.php')) {
    require __DIR__ . '/preload.php';
}

if (\Pnlx\Runtime::enableFunctions()) {
    $GLOBALS[$runtimeVarName] = $runtime->load({{FQCN}}::class);

{{FUNCTIONS}}
}

if (is_file(__DIR__ . '/postload.php')) {
    require __DIR__ . '/postload.php';
}

unset($runtimeVarName);
unset($runtime);
