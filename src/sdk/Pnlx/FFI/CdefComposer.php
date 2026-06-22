<?php

declare(strict_types=1);

namespace Pnlx\FFI;

/**
 * Merges several generated cdef strings into one, so {@see \Pnlx\Runtime::compose()}
 * can bind a single FFI scope shared by several extensions.
 *
 * The strategy is deliberately simple and order-preserving: the first member's
 * cdef is kept whole (it is expected to be the "base" library that fully defines
 * the shared types), then each later member contributes only the declarations
 * whose introduced C identifier is not already present. So a co-library that
 * forward-declares a shared type (`typedef struct SDL_Surface SDL_Surface;`) and
 * adds its own functions (`IMG_Load`) merges cleanly against a base that already
 * defines that type — without redeclaration errors.
 *
 * It is a pure string transform (no FFI), so it is unit-testable on its own.
 */
final class CdefComposer
{
    /**
     * @param list<string> $cdefs Member cdef strings, base first.
     */
    public static function merge(array $cdefs): string
    {
        $merged = array_shift($cdefs) ?? '';
        foreach ($cdefs as $cdef) {
            foreach (self::splitDeclarations($cdef) as $declaration) {
                $name = self::declarationName($declaration);
                if ($name !== null && preg_match('/\b' . preg_quote($name, '/') . '\b/', $merged) === 1) {
                    continue; // already defined by an earlier member
                }
                $merged .= "\n" . $declaration . ";\n";
            }
        }

        return $merged;
    }

    /**
     * Split a cdef into top-level declarations (brace-aware), with `/* *​/`
     * comments stripped and trailing semicolons removed.
     *
     * @return list<string>
     */
    public static function splitDeclarations(string $cdef): array
    {
        $cdef = preg_replace('#/\*.*?\*/#s', '', $cdef) ?? $cdef;

        $declarations = [];
        $buffer = '';
        $depth = 0;
        $length = strlen($cdef);
        for ($i = 0; $i < $length; $i++) {
            $char = $cdef[$i];
            if ($char === '{') {
                $depth++;
            } elseif ($char === '}') {
                $depth--;
            }
            if ($char === ';' && $depth === 0) {
                $declaration = trim($buffer);
                if ($declaration !== '') {
                    $declarations[] = $declaration;
                }
                $buffer = '';
                continue;
            }
            $buffer .= $char;
        }

        return $declarations;
    }

    /**
     * The C identifier a declaration introduces — a function name, or the alias of
     * a typedef/struct/union/enum. Best-effort, aimed at the small, regular cdefs
     * pnl generates; returns null when nothing identifier-like is found.
     */
    public static function declarationName(string $declaration): ?string
    {
        $withoutBody = preg_replace('/\{.*\}/s', '', $declaration) ?? $declaration;

        // A function declaration: the identifier immediately before the first '('
        // (typedefs are handled below, even function-pointer typedefs).
        if (!str_starts_with(ltrim($withoutBody), 'typedef')
            && preg_match('/([A-Za-z_]\w*)\s*\(/', $withoutBody, $matches) === 1
        ) {
            return $matches[1];
        }

        // Otherwise the last identifier token is the introduced name
        // (`typedef unsigned int Uint32` -> Uint32; `struct SDL_Rect { … } SDL_Rect`).
        if (preg_match_all('/[A-Za-z_]\w*/', $withoutBody, $matches) === false) {
            return null;
        }
        $tokens = $matches[0];

        return $tokens === [] ? null : (string) end($tokens);
    }
}
