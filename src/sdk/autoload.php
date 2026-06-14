<?php

declare(strict_types=1);

/*
 * Standalone loader for the pnl PHP SDK (the `Pnlx\` namespace).
 *
 * This is the single, canonical SDK autoloader used identically in every context:
 * it resolves `Pnlx\` classes relative to its OWN directory (`__DIR__/Pnlx/`). In
 * this repository it lives at `src/sdk/autoload.php` (→ `src/sdk/Pnlx/`); at
 * runtime the very same file is expanded to `@pnlx/runtime/autoload.php`
 * (→ `@pnlx/runtime/Pnlx/`) and required by the generated `@pnlx/autoload.php`.
 *
 * The SDK is intentionally NOT registered with any external autoloader, so it
 * stays self-contained. spl_autoload_register only appends to the autoload
 * queue (it never overrides an autoloader registered earlier); it simply
 * resolves the `Pnlx\` classes nothing else is told about.
 */

spl_autoload_register(static function (string $class): void {
    if (!str_starts_with($class, 'Pnlx\\')) {
        return;
    }

    $file = __DIR__ . '/Pnlx/' . str_replace('\\', '/', substr($class, 5)) . '.php';
    if (is_file($file)) {
        require $file;
    }
});

// Function definitions are not autoloadable; require them once (idempotent so the
// loader is safe to include more than once or alongside another SDK copy). The
// `Pnlx\Helpers\*` classes (the self-contained value-type layer) are resolved by
// the autoloader above, like the rest of the SDK. The `Pnlx\Util\*` functions
// (is_null + the wrapper-aware is_int/is_float/is_string/gettype helpers) live in
// one file.
if (!function_exists('Pnlx\\Util\\is_null')) {
    require_once __DIR__ . '/Pnlx/Util/functions.php';
}
