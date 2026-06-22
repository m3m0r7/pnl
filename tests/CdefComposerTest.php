<?php

declare(strict_types=1);

namespace Pnlx\Tests;

use PHPUnit\Framework\TestCase;
use Pnlx\FFI\CdefComposer;

class CdefComposerTest extends TestCase
{
    public function testSplitDeclarationsIsBraceAwareAndStripsComments(): void
    {
        $cdef = <<<'CDEF'
            /* a leading comment; with a semicolon inside */
            typedef unsigned int Uint32;
            typedef struct SDL_Rect { int x; int y; int w; int h; } SDL_Rect;
            int SDL_Init(Uint32 flags);
            CDEF;

        $declarations = CdefComposer::splitDeclarations($cdef);

        self::assertSame(
            [
                'typedef unsigned int Uint32',
                'typedef struct SDL_Rect { int x; int y; int w; int h; } SDL_Rect',
                'int SDL_Init(Uint32 flags)',
            ],
            $declarations,
        );
    }

    public function testDeclarationNameForFunctionsAndTypedefs(): void
    {
        self::assertSame('SDL_Init', CdefComposer::declarationName('int SDL_Init(Uint32 flags)'));
        self::assertSame('IMG_Load', CdefComposer::declarationName('SDL_Surface *IMG_Load(const char *file)'));
        self::assertSame('Uint32', CdefComposer::declarationName('typedef unsigned int Uint32'));
        self::assertSame('SDL_Rect', CdefComposer::declarationName('typedef struct SDL_Rect { int x; int y; } SDL_Rect'));
        self::assertSame('SDL_Surface', CdefComposer::declarationName('typedef struct SDL_Surface SDL_Surface'));
    }

    public function testMergeKeepsBaseAndAppendsOnlyUnknownDeclarations(): void
    {
        // The base fully defines SDL_Surface; the add-on forward-declares it and
        // adds IMG_Load. The forward decl must be dropped, IMG_Load kept.
        $base = "typedef struct SDL_Surface { int w; int h; } SDL_Surface;\n"
            . "int SDL_Init(unsigned int flags);\n";
        $addon = "typedef struct SDL_Surface SDL_Surface;\n"
            . "SDL_Surface *IMG_Load(const char *file);\n";

        $merged = CdefComposer::merge([$base, $addon]);

        // SDL_Surface defined exactly once (no redeclaration of the forward typedef).
        self::assertSame(1, substr_count($merged, 'typedef struct SDL_Surface'));
        // The add-on's new function was appended.
        self::assertStringContainsString('IMG_Load(const char *file)', $merged);
        // The base is preserved verbatim at the front.
        self::assertStringStartsWith($base, $merged);
    }

    public function testMergeOfSingleCdefReturnsItUnchanged(): void
    {
        $only = "int SDL_Init(unsigned int flags);\n";
        self::assertSame($only, CdefComposer::merge([$only]));
    }
}
