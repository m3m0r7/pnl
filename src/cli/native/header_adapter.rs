use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use clang::token::TokenKind;
use clang::{Clang, Entity, EntityKind, Index, Linkage, TypeKind};

mod const_eval;
mod macros;
mod render;

use const_eval::{ConstValue, IntKind, definition_const_value, eval_const, render_const_value};
use macros::{macro_functions, macro_symbol_aliases};
use render::{
    builtin_type_names, contains_c_identifier, filter_verbatim_to_exports, global_decl_name,
    is_c_identifier_char, render, split_declaration_name,
};

/// libclang holds process-global state and the `clang` crate only permits a
/// single live [`Clang`] token at a time, so every translation is serialised.
static CLANG_GUARD: Mutex<()> = Mutex::new(());

/// Self-contained type aliases prepended to the generated cdef. PHP's
/// `FFI::cdef` has no access to system headers, so these stand in for the
/// fixed-width and SDL-style integer types the real headers rely on. The same
/// text is fed to libclang while parsing so those names resolve there too.
///
/// The list lives in a template file so it sits alongside the other generated
/// artifacts rather than being buried in Rust source.
const PRELUDE: &str = include_str!("../templates/header/prelude.h");

/// A lexer token kept for macro analysis: its kind and its spelling.
type Token = (TokenKind, String);

#[derive(Debug, Clone, Default)]
pub struct HeaderAdapterOptions {
    pub symbol_prefix: String,
    /// The generated entity's fully-qualified class name (e.g. `\Pnlx\Libsdl\Libsdl`),
    /// used to render the C-function calls inside function-like macros.
    pub entity_fqcn: String,
    /// `C function name -> dependency entity FQCN` for functions this library does
    /// not define but a (recursive) `dependencies` package does, so a macro that
    /// calls one renders a static call to the dependency instead of throwing.
    pub dependency_functions: BTreeMap<String, String>,
    /// When set, cdef function declarations are limited to these exported
    /// symbols (so the installed library's version/build skew can't fail the
    /// FFI load). `None` keeps every declaration.
    pub exported_symbols: Option<BTreeSet<String>>,
    /// The package's resolved header file paths. Their directory (and a
    /// subdirectory named after the umbrella header, e.g. `sodium.h` ->
    /// `sodium/`) define which `#include`d headers are part of *this* package,
    /// so functions split across the package's own sub-headers are collected
    /// while system / other-library headers are not.
    pub package_header_paths: Vec<PathBuf>,
    /// Extra `-I` include directories for the libclang parse, from the library's
    /// `pkg-config --cflags`. These cover devel headers that live outside a header's
    /// own ancestry — notably libdir configs like GLib's `glibconfig.h` and
    /// `pango-features.h` — so their types and version-gate macros resolve (instead
    /// of being undefined, which drops `*_AVAILABLE_IN_*`-decorated functions).
    pub extra_include_dirs: Vec<PathBuf>,
    /// `require_definitions` resolved at install time (e.g. pcre2's
    /// `PCRE2_CODE_UNIT_WIDTH=8`). Each is passed to libclang as a `-D` (so a
    /// config-gated header parses and its width-suffixed symbols are collected) and
    /// emitted as a generated constant.
    pub definitions: Vec<crate::model::manifest::ResolvedDefinition>,
}

/// A function-like macro turned into a PHP function (in `\Pnlx\Func\<Class>`).
#[derive(Debug)]
pub struct MacroFunction {
    pub name: String,
    pub params: Vec<String>,
    /// The PHP body: `Ok(expr)` for `return <expr>;`, or `Err(symbol)` for a
    /// function that throws because it calls the undefined C function `symbol`.
    pub body: std::result::Result<String, String>,
}

/// Everything extracted from a C header: the cdef text for PHP `FFI::cdef`, the
/// object-like `#define` constants (for `const.php`), and the function-like
/// macros turned into PHP functions.
#[derive(Debug, Default)]
pub struct HeaderArtifacts {
    pub cdef: String,
    /// The object-like `#define` (and enum) constants, in source order, for
    /// `const.php` and its `scalar/` variant.
    pub constants: Vec<Constant>,
    pub macro_functions: Vec<MacroFunction>,
    /// Exported data symbols (C globals), for generating per-symbol marker classes.
    pub symbols: Vec<DataSymbol>,
    /// `(public_name, native_symbol)` for object-like macros that rename an export
    /// back to its unversioned public name. ICU ships `#define u_errorName
    /// U_ICU_ENTRY_POINT_RENAME(u_errorName)`, which the preprocessor expands to the
    /// versioned symbol `u_errorName_74`; the generated method keeps the public name
    /// while dispatching to the versioned symbol the library actually exports.
    pub symbol_aliases: Vec<(String, String)>,
    /// Functions that cannot be bound through FFI but are still surfaced as a
    /// generated method that throws (today: `static inline`, which has no exported
    /// symbol). Kept out of the cdef.
    pub unsupported_functions: Vec<UnsupportedFunction>,
    /// Named C enums surfaced as PHP `enum`s (see [`EnumDef`]).
    pub enums: Vec<EnumDef>,
}

/// A function with no FFI binding, surfaced as a throwing stub method. `declaration`
/// is the same `ret name(params);` form as a real function (so its signature/types
/// render), and `reason` is an abstract category (e.g. `static inline`) used for the
/// thrown message and the generated marker attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFunction {
    pub declaration: String,
    pub reason: String,
}

/// One exported data symbol (a C global variable). `pointer` is true when the
/// symbol's own type is already a pointer (the API wants its value, e.g.
/// `OnigDefaultSyntax`); false for a value the API takes the address of (e.g. the
/// struct instance `OnigEncodingUTF8`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSymbol {
    pub name: String,
    pub pointer: bool,
}

/// One emitted `const.php` constant, rendered in both forms the two const
/// variants need: `wrapped` is a `\Pnlx\Types\*` object (the default), `scalar`
/// is a bare PHP `int`/`float`/`string` when the value is losslessly
/// representable (the `scalar/` variant) and falls back to the wrapped form for
/// typed/unsigned values that must not be flattened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constant {
    pub name: String,
    pub wrapped: String,
    pub scalar: String,
}

/// A named C `enum`, surfaced as a real PHP `enum <name>: int`. `cases` are the
/// enumerators in source order. Only emitted for a *named* enum whose case values
/// are all distinct (PHP enums forbid two cases sharing a value); an enum with
/// duplicate values stays projected to `int` and its enumerators remain in
/// `const.php`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDef {
    pub name: String,
    pub cases: Vec<(String, i64)>,
}

/// Translate a C header into a normalised cdef suitable for PHP `FFI::cdef`,
/// keeping only the declarations whose names contain `symbol_prefix`, and
/// extract its object-like `#define` constants.
///
/// Parsing is delegated to libclang; only declaration discovery and the
/// FFI-specific re-emission are performed here.
/// Whether libclang can be loaded right now — used by `pnl -i` to report toolchain
/// readiness without attempting a full header parse.
pub fn libclang_available() -> bool {
    ensure_libclang_available().is_ok()
}

/// Preflight that libclang can be loaded, returning the actionable, platform-specific
/// message on failure. Used to fail fast — before any download or native-library
/// resolution — when the one hard requirement for reading C headers is missing.
/// `cdef_from_header` still loads its own instance for the actual parse.
pub fn ensure_libclang_available() -> Result<()> {
    let _guard = CLANG_GUARD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    Clang::new()
        .map(|_| ())
        .map_err(|err| anyhow!("{}", libclang_unavailable_message(&err)))
}

/// An actionable, platform-specific error for when libclang cannot be loaded.
/// libclang is the one hard external requirement for generating an extension from
/// C headers (pnl loads it at runtime rather than linking it), so the message tells
/// the user exactly what to install.
pub fn libclang_unavailable_message(err: &impl std::fmt::Display) -> String {
    let mut message =
        format!("could not load libclang, which pnl needs to read C headers: {err}\n\n");
    if cfg!(target_os = "macos") {
        message.push_str(
            "Install the Xcode Command Line Tools (they bundle libclang):\n\
             \n\
             \txcode-select --install\n\
             \n\
             If they are already installed, point pnl at libclang with LIBCLANG_PATH,\n\
             e.g. export LIBCLANG_PATH=\"$(xcode-select -p)/Toolchains/XcodeDefault.xctoolchain/usr/lib\".",
        );
    } else {
        message.push_str(
            "Install your platform's libclang (it ships with the LLVM/Clang toolchain):\n\
             \n\
             \tDebian/Ubuntu : apt install libclang-dev\n\
             \tFedora/RHEL   : dnf install clang-devel\n\
             \tAlpine        : apk add clang-dev\n\
             \tArch          : pacman -S clang\n\
             \n\
             A C compiler (cc/gcc/clang) is also recommended so library search paths\n\
             resolve fully. If libclang is installed elsewhere, set LIBCLANG_PATH to its directory.",
        );
    }
    message
}

pub fn cdef_from_header(header: &str, options: &HeaderAdapterOptions) -> Result<HeaderArtifacts> {
    let prefix = options.symbol_prefix.trim();
    if prefix.is_empty() {
        // An empty symbol prefix means "use the header verbatim" (a curated
        // `header_inline`, e.g. libc) without libclang. But still drop any
        // single-line function declaration the resolved library does not export
        // (glibc's `atexit`/`at_quick_exit` live in `libc_nonshared.a`, not
        // `libc.so.6`): PHP FFI resolves every declared function eagerly, so one
        // unexported symbol fails the whole `FFI::cdef`. When no export set is known
        // (a virtual lib with no on-disk file, e.g. libc on macOS) the header is
        // used as-is.
        let cdef = match options.exported_symbols.as_ref() {
            Some(symbols) => filter_verbatim_to_exports(header, symbols),
            None => header.to_owned(),
        };
        return Ok(HeaderArtifacts {
            cdef,
            ..HeaderArtifacts::default()
        });
    }

    let _guard = CLANG_GUARD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let clang = Clang::new().map_err(|err| anyhow!("{}", libclang_unavailable_message(&err)))?;
    let index = Index::new(&clang, false, false);
    let workspace = Workspace::new()?;
    let prelude_path = workspace.write("prelude.h", PRELUDE)?;

    let owned_dirs = owned_package_dirs(&options.package_header_paths);
    let mut include_dirs = include_search_dirs(&options.package_header_paths);
    // The original headers are inlined into a temp `header.h`, so libclang can no
    // longer resolve their sibling quote-includes (`#include "unitypes.h"`)
    // relative to the original location. Add each resolved header's own directory
    // so those siblings resolve — they frequently define the very macros/types the
    // API declarations depend on (libunistring's `_UC_ATTRIBUTE_CONST` and `ucs4_t`
    // live in unitypes.h next to unictype.h; without it every `uc_is_*` decl drops).
    for path in &options.package_header_paths {
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            // The header's own directory, for sibling quote-includes.
            if !include_dirs.contains(&parent) {
                include_dirs.push(parent.clone());
            }
            // When the header lives under an `include/<pkg>/` subdirectory, also
            // add that `include` root so an angle include carrying the subdir
            // prefix resolves (lzo's `#include <lzo/lzodefs.h>` needs
            // `.../include` on `-I`, not just `.../include/lzo`; without it
            // libclang aborts the parse and emits zero functions).
            if let Some(grandparent) = parent.parent().map(Path::to_path_buf)
                && grandparent
                    .file_name()
                    .is_some_and(|name| name == "include")
                && !include_dirs.contains(&grandparent)
            {
                include_dirs.push(grandparent);
            }
        }
    }
    // The pkg-config `--cflags` dirs (libdir configs etc.) the compiler would see.
    for dir in &options.extra_include_dirs {
        if !include_dirs.contains(dir) {
            include_dirs.push(dir.clone());
        }
    }
    // `-D` flags for the user-resolved `require_definitions`, so a config-gated
    // header (pcre2's `PCRE2_CODE_UNIT_WIDTH`) parses and its width-suffixed symbols
    // resolve. A string definition is quoted; everything else is passed bare.
    let define_args: Vec<String> = options
        .definitions
        .iter()
        .map(|definition| match definition.definition_type {
            crate::model::manifest::DefinitionType::String => {
                format!("-D{}=\"{}\"", definition.name, definition.value)
            }
            _ => format!("-D{}={}", definition.name, definition.value),
        })
        .collect();
    let defined_names: BTreeSet<String> = options
        .definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect();
    // Headers reached via quoted includes are part of the package's own API even
    // when they sit as siblings directly under a system include root (z3's
    // `z3_api.h`, hdf5's `H5public.h`); mark them owned so their declarations are
    // emitted rather than silently dropped, which would leave an empty binding.
    let owned_files = owned_header_files(&options.package_header_paths, &include_dirs);
    let collected = parse_with_neutralized_macros(
        &index,
        &workspace,
        &prelude_path,
        header,
        &owned_dirs,
        &owned_files,
        &include_dirs,
        &define_args,
        &defined_names,
    )?;
    let constants = header_constants(&collected, prefix, &options.definitions);
    let constant_names: BTreeSet<String> = constants
        .iter()
        .map(|constant| constant.name.clone())
        .collect();
    Ok(HeaderArtifacts {
        cdef: render(&collected, options.exported_symbols.as_ref()),
        macro_functions: macro_functions(&collected, prefix, &constant_names, options),
        symbols: data_symbols(&collected, options.exported_symbols.as_ref()),
        symbol_aliases: macro_symbol_aliases(&collected, &options.definitions),
        unsupported_functions: unsupported_functions(&collected),
        enums: enum_definitions(&collected, prefix),
        constants,
    })
}

/// The named C enums to surface as PHP enums: package-owned, prefix-matching (the
/// tag carries the symbol prefix), with all-distinct case values (PHP enums forbid
/// duplicate-valued cases). Others stay projected to `int` with their enumerators in
/// `const.php`.
fn enum_definitions(collected: &Collected, prefix: &str) -> Vec<EnumDef> {
    collected
        .enum_definitions
        .iter()
        .filter(|def| def.name.contains(prefix))
        .filter(|def| {
            let mut seen = BTreeSet::new();
            def.cases.iter().all(|(_, value)| seen.insert(*value))
        })
        .cloned()
        .collect()
}

/// The unbindable functions (today: `static inline`) to surface as throwing stub
/// methods. Like the cdef's real declarations they are package-owned; the
/// exported-symbols filter does not apply (a `static inline` is, by definition, not
/// an export).
fn unsupported_functions(collected: &Collected) -> Vec<UnsupportedFunction> {
    collected
        .unsupported_functions
        .iter()
        .map(|(declaration, reason)| UnsupportedFunction {
            declaration: declaration.clone(),
            reason: reason.clone(),
        })
        .collect()
}

/// The exported data symbols to surface as marker classes, filtered by the
/// installed library's exports (same guard as the cdef's `extern` declarations).
fn data_symbols(
    collected: &Collected,
    exported_symbols: Option<&BTreeSet<String>>,
) -> Vec<DataSymbol> {
    collected
        .globals
        .iter()
        .filter_map(|global| {
            let name = global_decl_name(global)?;
            if exported_symbols.is_some_and(|symbols| !symbols.contains(&name)) {
                return None;
            }
            Some(DataSymbol {
                pointer: global.contains('*'),
                name,
            })
        })
        .collect()
}

/// Parse the header, neutralising the ABI/attribute macros (e.g. `DECLSPEC`,
/// `SDLCALL`, `LIBUSB_CALL`, `LIBUSB_PACKED`) that appear in unpreprocessed
/// headers but whose definitions live in headers we are not given. They are
/// replaced with empty `#define`s so libclang can parse the declarations.
///
/// Candidates are found two ways: tokens that look like ABI macros (all-caps
/// call-convention / attribute names), and any remaining `unknown type name`
/// libclang reports. Enum constants, reserved/compiler macros, and any macro the
/// header `#define`s itself (include guards, value macros) are never neutralised,
/// so only genuinely-external macros are stubbed out.
#[allow(clippy::too_many_arguments)]
fn parse_with_neutralized_macros(
    index: &Index<'_>,
    workspace: &Workspace,
    prelude_path: &Path,
    header: &str,
    owned_dirs: &[PathBuf],
    owned_files: &BTreeSet<PathBuf>,
    include_dirs: &[PathBuf],
    define_args: &[String],
    defined_names: &BTreeSet<String>,
) -> Result<Collected> {
    let mut arguments = vec![
        "-x".to_owned(),
        "c".to_owned(),
        "-std=c11".to_owned(),
        "-ferror-limit=0".to_owned(),
        // Keep `#define`s in the AST as MacroDefinition cursors so object-like
        // constant macros can be re-emitted as PHP `const`s.
        "-Xclang".to_owned(),
        "-detailed-preprocessing-record".to_owned(),
        "-include".to_owned(),
        prelude_path
            .to_str()
            .context("prelude path is not UTF-8")?
            .to_owned(),
    ];
    // On macOS, point libclang at the active SDK so system headers
    // (`<time.h>`, `<sys/types.h>`, …) resolve. Without a sysroot, types like
    // `time_t`/`size_t` brought in by a `#include <…>` are undefined and
    // libclang's error recovery rewrites a callback typedef whose return type is
    // that undefined name into a bogus function type (`typedef int time_t(int*)`),
    // which then makes every function returning it look like "function returning
    // function" and fails the PHP-FFI parse (libgnutls, libenet).
    arguments.extend(macos_isysroot_args());
    // User-resolved `require_definitions`, so a config-gated header parses (pcre2's
    // `#error` guard) and its conditioned declarations/types are collected.
    arguments.extend(define_args.iter().cloned());
    // Add the package's own include roots so `#include <libxml/xmlstring.h>`
    // style directives inside the concatenated headers resolve to the real
    // sub-headers (otherwise types defined only there, e.g. `xmlChar`, are
    // undefined and collapse to `int`). Only dirs nested below a system include
    // root are added, so `/usr/include` packages are unaffected.
    for dir in include_dirs {
        if let Some(dir) = dir.to_str() {
            arguments.push(format!("-I{dir}"));
        }
    }
    let parse = |source: &str| -> Result<_> {
        let header_path = workspace.write("header.h", source)?;
        index
            .parser(&header_path)
            .arguments(&arguments)
            .skip_function_bodies(true)
            .parse()
            .context("libclang failed to parse the header")
    };

    // A lenient first pass: enums and typedefs parse even while ABI macros are
    // undefined, giving us the enum-constant and typedef names that must be excluded
    // from neutralisation (an enum constant, or an uppercase typedef like gdbm's
    // `GDBM_FILE`, otherwise looks like an attribute macro and gets stubbed empty).
    let lenient = parse(header)?;
    let lenient_root = lenient.get_entity();
    let enum_constants = gather_enum_constants(&lenient_root);
    let typedef_names = gather_typedef_names(&lenient_root);
    // A user-resolved `require_definitions` macro (passed as a `-D`) must never be
    // neutralised: an empty redefinition would override the `-D` value and flip the
    // header's `#if` width gates, dropping the conditioned declarations.
    let mut defines: BTreeSet<String> = abi_macro_candidates(header)
        .difference(&enum_constants)
        .filter(|name| !defined_names.contains(*name) && !typedef_names.contains(*name))
        .cloned()
        .collect();
    let mut opaque_types: BTreeMap<String, &'static str> = BTreeMap::new();

    for _ in 0..32 {
        let unit = parse(&neutralized_source(header, &defines, &opaque_types))?;

        let mut discovered = false;
        for diagnostic in unit.get_diagnostics() {
            if let Some(name) = unknown_type_name(&diagnostic.get_text())
                && !enum_constants.contains(&name)
            {
                if let Some(kind) = declared_aggregate_kind(header, &name) {
                    if opaque_types.insert(name, kind).is_none() {
                        discovered = true;
                    }
                } else if !defined_names.contains(&name) && defines.insert(name) {
                    discovered = true;
                }
            }
        }

        if !discovered {
            let main_header = workspace.root.join("header.h");
            return Ok(collect(
                &unit.get_entity(),
                &main_header,
                owned_dirs,
                owned_files,
            ));
        }
    }

    Err(anyhow!(
        "header still failed to parse after neutralising macros"
    ))
}

/// `-isysroot <path>` for the active macOS SDK so libclang resolves system
/// headers, or empty off macOS / when no SDK can be located. `$SDKROOT` wins
/// (honours an explicit override), then `xcrun --show-sdk-path`, then a
/// developer-dir fallback. Mirrors `native.rs`'s `macos_sdk_lib_dirs`.
pub(crate) fn macos_isysroot_args() -> Vec<String> {
    if std::env::consts::OS != "macos" {
        return Vec::new();
    }
    let run = |program: &str, args: &[&str]| -> Option<String> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let sdk = std::env::var("SDKROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| run("xcrun", &["--show-sdk-path"]).filter(|s| !s.is_empty()))
        .or_else(|| {
            run("xcode-select", &["-p"])
                .filter(|s| !s.is_empty())
                .map(|dev| format!("{dev}/SDKs/MacOSX.sdk"))
        });
    match sdk {
        Some(sdk) if Path::new(&sdk).is_dir() => vec!["-isysroot".to_owned(), sdk],
        _ => Vec::new(),
    }
}

/// Identifier tokens that look like call-convention or attribute macros:
/// all-caps names containing an underscore or `CALL`, plus bare `DECLSPEC`.
fn abi_macro_candidates(header: &str) -> BTreeSet<String> {
    // Macros the header `#define`s itself must NOT be stubbed: include guards
    // (`#ifndef LIBUSB_H`/`#define LIBUSB_H`) would otherwise mark the whole header
    // as already-included and skip every declaration, and value macros
    // (`#define LIBUSB_API_VERSION 0x…`) feed `#if` expressions. The header's own
    // (possibly platform-conditional) definition is the right one.
    let defined = header_defined_macros(header);
    // Macros used in preprocessor conditionals (`#ifndef Z_SOLO`, `#if
    // defined(FOO)`) are feature/config gates, not ABI annotations. Stubbing
    // one with an empty `#define` would flip the condition and silently drop
    // whole `#ifndef`-guarded blocks (e.g. zlib's `compress`/`gz*` functions).
    let conditional = header_conditional_macros(header);
    let mut candidates = BTreeSet::new();
    for token in identifier_tokens(header) {
        if is_abi_macro_token(token) && !defined.contains(token) && !conditional.contains(token) {
            candidates.insert(token.to_owned());
        }
    }
    candidates
}

/// Identifiers that appear in preprocessor conditional directives (`#if`,
/// `#ifdef`, `#ifndef`, `#elif`, and `defined(...)`). These gate compilation and
/// must never be neutralised.
fn header_conditional_macros(header: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in header.lines() {
        let rest = line.trim_start();
        let Some(rest) = rest.strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        let directive = rest
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .next()
            .unwrap_or("");
        let operands = match directive {
            "ifdef" | "ifndef" | "if" | "elif" => &rest[directive.len()..],
            _ => continue,
        };
        for token in identifier_tokens(operands) {
            if token != "defined" {
                names.insert(token.to_owned());
            }
        }
    }
    names
}

/// Names introduced by `#define NAME …` anywhere in the header.
fn header_defined_macros(header: &str) -> BTreeSet<String> {
    let mut defined = BTreeSet::new();
    for line in header.lines() {
        let rest = line.trim_start();
        let Some(rest) = rest.strip_prefix('#') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("define") else {
            continue;
        };
        let name: String = rest
            .trim_start()
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            defined.insert(name);
        }
    }
    defined
}

fn is_abi_macro_token(token: &str) -> bool {
    // Reserved identifiers (`__GNUC__`, `_WIN32`, `_MSC_VER`, …) are compiler and
    // feature-test macros, not ABI annotations. Stubbing them to empty breaks the
    // header's own preprocessor logic — and *defining* a platform macro like
    // `_WIN32` would switch on the Windows-only branches (e.g. `#include
    // <basetsd.h>`), so a header's functions silently vanish.
    if token.starts_with('_') {
        return false;
    }
    if token == "DECLSPEC" {
        return true;
    }
    let all_caps = token.len() >= 2
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        && token.chars().any(|ch| ch.is_ascii_uppercase());
    all_caps && (token.contains('_') || token.contains("CALL"))
}

fn identifier_tokens(input: &str) -> impl Iterator<Item = &str> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|token| {
            !token.is_empty() && token.chars().next().is_some_and(|ch| !ch.is_ascii_digit())
        })
}

fn gather_enum_constants(translation_unit: &Entity<'_>) -> BTreeSet<String> {
    let mut constants = BTreeSet::new();
    for entity in translation_unit.get_children() {
        if entity.get_kind() == EntityKind::EnumDecl {
            for constant in entity.get_children() {
                if constant.get_kind() == EntityKind::EnumConstantDecl
                    && let Some(name) = constant.get_name()
                {
                    constants.insert(name);
                }
            }
        }
    }
    constants
}

/// Typedef names declared in the header, gathered from the lenient (un-neutralised)
/// first pass. They must be excluded from ABI-macro neutralisation: an all-uppercase
/// typedef such as gdbm's `GDBM_FILE` matches the attribute-macro heuristic, and
/// stubbing it with an empty `#define` erases the typedef name (`typedef struct … *;`
/// becomes nameless and is dropped), mistyping every use to `int`.
fn gather_typedef_names(translation_unit: &Entity<'_>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entity in translation_unit.get_children() {
        if entity.get_kind() == EntityKind::TypedefDecl
            && let Some(name) = entity.get_name()
        {
            names.insert(name);
        }
    }
    names
}

/// Whether `name` is ever invoked as a function-like macro in the header — i.e.
/// appears as a whole token immediately (modulo whitespace) followed by `(`. Used
/// to choose a passthrough stub over an empty object-like one when neutralising it.
fn used_function_like(header: &str, name: &str) -> bool {
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = header.as_bytes();
    let mut from = 0;
    while let Some(offset) = header[from..].find(name) {
        let start = from + offset;
        let end = start + name.len();
        from = end;
        // Reject a match that is part of a longer identifier on either side.
        if start > 0 && is_ident(bytes[start - 1]) {
            continue;
        }
        if end < bytes.len() && is_ident(bytes[end]) {
            continue;
        }
        let mut cursor = end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'(' {
            return true;
        }
    }
    false
}

fn neutralized_source(
    header: &str,
    defines: &BTreeSet<String>,
    opaque_types: &BTreeMap<String, &'static str>,
) -> String {
    let mut source = String::new();
    for name in defines {
        source.push_str("#define ");
        source.push_str(name);
        // A macro invoked function-like — wrapping the return type (`FT_EXPORT( T )`)
        // or the parameter list (zlib's `OF((args))`) — must be stubbed as a
        // passthrough that yields its arguments, or the leftover parentheses break
        // the declaration. An object-like stub is right for the rest (`ZEXTERN`).
        if used_function_like(header, name) {
            source.push_str("(...) __VA_ARGS__");
        }
        source.push('\n');
    }
    for (name, kind) in opaque_types {
        source.push_str("typedef ");
        source.push_str(kind);
        source.push(' ');
        source.push_str(name);
        source.push(' ');
        source.push_str(name);
        source.push_str(";\n");
    }
    source.push_str(header);
    source
}

fn declared_aggregate_kind(header: &str, name: &str) -> Option<&'static str> {
    if contains_c_identifier(header, &format!("typedef struct {name}"))
        || contains_c_identifier(header, &format!("struct {name}"))
    {
        return Some("struct");
    }
    if contains_c_identifier(header, &format!("typedef union {name}"))
        || contains_c_identifier(header, &format!("union {name}"))
    {
        return Some("union");
    }
    None
}

/// Extract `unknown type name 'X'` from a diagnostic message.
fn unknown_type_name(message: &str) -> Option<String> {
    let rest = message.strip_prefix("unknown type name ")?;
    rest.trim()
        .strip_prefix('\'')?
        .split('\'')
        .next()
        .map(str::to_owned)
}

/// An object-like `#define` macro: its name and replacement tokens (kind +
/// spelling), captured in header source order.
struct RawMacro {
    name: String,
    tokens: Vec<Token>,
}

/// A function-like `#define NAME(params) body`. Not emitted as a constant, but
/// kept so an object-like macro that *invokes* it with constant arguments can be
/// expanded (e.g. `#define X DISPLAY(0)`).
struct RawFnMacro {
    params: Vec<String>,
    body: Vec<Token>,
}

#[derive(Default)]
struct Collected {
    structs: BTreeMap<String, String>,
    struct_aliases: BTreeMap<String, String>,
    unions: BTreeMap<String, String>,
    union_aliases: BTreeMap<String, String>,
    enums: BTreeSet<String>,
    typedef_names: BTreeSet<String>,
    typedefs: Vec<String>,
    functions: Vec<String>,
    /// Functions that cannot be bound through FFI (today: `static inline`, which has
    /// no exported symbol) but are still surfaced as a generated method that throws,
    /// so the API is complete and calling one gives a clear error instead of a
    /// "method not found". Each is a `(declaration, reason)`; the declaration is the
    /// same `ret name(params);` form as `functions` but is never put in the cdef.
    unsupported_functions: Vec<(String, String)>,
    /// Names already collected as unsupported, to dedupe redeclarations.
    unsupported_names: BTreeSet<String>,
    /// Exported global variables (`extern <type> <name>;`), so a PHP example can
    /// take their address through `NativeLibrary::addressOf()` for an API that wants
    /// a pointer to a global (oniguruma's `ONIG_ENCODING_UTF8` = `&OnigEncodingUTF8`).
    globals: Vec<String>,
    macros: Vec<RawMacro>,
    /// Function-like macros by name, for constant-argument expansion.
    fn_macros: BTreeMap<String, RawFnMacro>,
    /// Names of the kept (externally-linked) C functions, for resolving the
    /// calls inside function-like macros.
    function_names: BTreeSet<String>,
    /// `(name, value)` for every enumerator, in source order, for `const.php`.
    enum_constants: Vec<(String, i64)>,
    /// Named enums with their cases, for emitting PHP `enum`s (see [`EnumDef`]).
    enum_definitions: Vec<EnumDef>,
    /// Enumerators from #included (non-owned) headers, e.g. libtidy's
    /// `TidyOptionId` values in `tidyenum.h` when it sits flat in `/usr/include`
    /// (Alpine) rather than a package subdir (Ubuntu's `/usr/include/tidy/`).
    /// Only the symbol-prefix-matching ones are surfaced in `const.php`, so this
    /// can't drag in unrelated system enumerators.
    pool_enum_constants: Vec<(String, i64)>,
    /// Plain typedefs (`name -> underlying display`) discovered in *included*
    /// headers (e.g. `voidpf`, `uLong` from zlib's `zconf.h`). Not emitted
    /// wholesale; only those a kept declaration transitively references are
    /// resolved into the cdef (see `resolve_pool_types`).
    pool_typedefs: BTreeMap<String, String>,
    /// Record/enum typedefs from included headers: `alias -> struct/union tag`,
    /// and bare enum-typedef names projected onto `int`.
    pool_struct_aliases: BTreeMap<String, String>,
    pool_union_aliases: BTreeMap<String, String>,
    pool_enums: BTreeSet<String>,
}

/// Whether a declaration belongs to the package's own header rather than an
/// `#include`d one. Unlike `Entity::is_in_main_file`, this uses the *expansion*
/// location, so a function declared through macros (e.g. zlib's `ZEXTERN ret
/// ZEXPORT name OF((args))`, or libbz2's `BZ_API(name)`) is still recognised as
/// belonging to the main file instead of being misattributed to the macro body.
fn entity_owned(
    entity: &Entity<'_>,
    main_header: &Path,
    owned_dirs: &[PathBuf],
    owned_files: &BTreeSet<PathBuf>,
) -> bool {
    let Some(path) = entity
        .get_location()
        .and_then(|location| location.get_expansion_location().file)
        .map(|file| file.get_path())
    else {
        return false;
    };
    // The concatenated package headers are written to a temp `header.h`; match it
    // by leaf name (libclang may normalise the directory, e.g. `/private/var`).
    if path.file_name() == main_header.file_name() {
        return true;
    }
    // A header the package pulls in via a quoted include (its own API), even one
    // living as a sibling directly under a system include root — z3's `z3_api.h`,
    // hdf5's `H5public.h` — which `owned_dirs` can't capture without also claiming
    // unrelated system headers in the same directory.
    let canonical = std::fs::canonicalize(&path).ok();
    if owned_files.contains(&path)
        || canonical
            .as_deref()
            .is_some_and(|real| owned_files.contains(real))
    {
        return true;
    }
    // A header can be reached through several symlink spellings of the same real
    // directory: Homebrew exposes the same Cellar dir as `/opt/homebrew/include/<pkg>`,
    // `/opt/homebrew/opt/<pkg>/include/<pkg>`, and the Cellar path itself, and
    // libclang reports whichever `-I` spelling it used. Compare both the raw path
    // and its symlink-resolved form against the (also symlink-augmented) owned
    // dirs, so a sub-header declaration (jasper's `jas_getversion`) is recognised
    // regardless of which spelling each side happens to use.
    owned_dirs.iter().any(|dir| {
        path.starts_with(dir)
            || canonical
                .as_deref()
                .is_some_and(|real| real.starts_with(dir))
    })
}

/// Directories whose headers count as part of this package: each resolved
/// header's own directory (unless it's a shared system include root) plus a
/// subdirectory named after the umbrella header (`sodium.h` -> `sodium/`),
/// where libraries keep their split-out sub-headers.
fn owned_package_dirs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    // Add `dir` and, when it differs, its symlink-resolved real path. Homebrew
    // exposes headers through `/opt/homebrew/include/<pkg>` symlinks into
    // `…/Cellar/<pkg>/<ver>/include/<pkg>`, but libclang reports the canonical
    // Cellar path for `#include`d sub-headers. Without the canonical variant,
    // `entity_owned`'s `starts_with` check drops every sub-header declaration
    // (e.g. jasper's `jas_getversion`, libfido2's `fido_strerr`).
    let push = |dir: PathBuf, dirs: &mut Vec<PathBuf>| {
        if !dirs.contains(&dir) {
            dirs.push(dir.clone());
        }
        if let Ok(canonical) = std::fs::canonicalize(&dir)
            && canonical != dir
            && !dirs.contains(&canonical)
        {
            dirs.push(canonical);
        }
    };
    for path in paths {
        let Some(parent) = path.parent() else {
            continue;
        };
        if !is_system_include_root(parent) {
            push(parent.to_path_buf(), &mut dirs);
        }
        if let Some(stem) = path.file_stem() {
            push(parent.join(stem), &mut dirs);
        }
    }
    dirs
}

/// The header files a package owns beyond its declared entry headers: every header
/// reached through a *quoted* `#include "..."` (a project's own headers, by
/// convention), resolved transitively. This matters when a library splits its API
/// across sibling headers that live directly under a system include root — z3's
/// `z3_api.h`, hdf5's `H5public.h` in `/usr/include` — whose declarations would
/// otherwise be dropped as "not owned" (a sibling can't be a sub-directory of the
/// entry header's stem dir, and the dir itself is a shared system root). Angle
/// includes are skipped: those are system/other-library headers (`<stdint.h>`), and
/// genuine nested package roots are already covered by `include_search_dirs`.
fn owned_header_files(
    package_header_paths: &[PathBuf],
    include_dirs: &[PathBuf],
) -> BTreeSet<PathBuf> {
    let mut owned: BTreeSet<PathBuf> = BTreeSet::new();
    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<PathBuf> = package_header_paths.to_vec();
    while let Some(path) = queue.pop() {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !visited.insert(key) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let dir = path.parent().map(Path::to_path_buf);
        for include in quoted_includes(&source) {
            let candidates = dir
                .iter()
                .map(|d| d.join(&include))
                .chain(include_dirs.iter().map(|d| d.join(&include)));
            for candidate in candidates {
                if candidate.is_file() {
                    if let Ok(canonical) = std::fs::canonicalize(&candidate) {
                        owned.insert(canonical);
                    }
                    owned.insert(candidate.clone());
                    queue.push(candidate);
                    break;
                }
            }
        }
    }
    owned
}

/// Header names from `#include "name"` (quoted) directives only; `<…>` are ignored.
fn quoted_includes(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("include") else {
            continue;
        };
        if let Some(rest) = rest.trim_start().strip_prefix('"')
            && let Some(end) = rest.find('"')
        {
            names.push(rest[..end].to_owned());
        }
    }
    names
}

/// `-I` search roots for the package's nested headers. For a resolved header
/// such as `/usr/include/libxml2/libxml/parser.h`, libclang's default search
/// path covers `/usr/include` but not `/usr/include/libxml2`, so an internal
/// `#include <libxml/xmlstring.h>` fails and types defined only there collapse
/// to `int`. Each ancestor directory between the header and the first system
/// include root (exclusive) is returned, so packages laid out directly under
/// `/usr/include` (already on the default path) gain no extra roots.
///
/// Only headers that actually live below a system include root contribute roots;
/// a header elsewhere (a test fixture, a `self_build` checkout) yields none,
/// rather than walking up to `/` and feeding libclang bogus `-I` paths.
pub(crate) fn include_search_dirs(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for path in paths {
        let mut pending: Vec<PathBuf> = Vec::new();
        let mut current = path.parent();
        let mut under_system_root = false;
        while let Some(dir) = current {
            if is_system_include_root(dir) {
                under_system_root = true;
                break;
            }
            pending.push(dir.to_path_buf());
            current = dir.parent();
        }
        if under_system_root {
            for dir in pending {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    dirs
}

/// A top-level include directory shared with the C library and system headers,
/// so it must not, on its own, mark every header inside it as package-owned.
fn is_system_include_root(dir: &Path) -> bool {
    let path = dir.to_string_lossy();
    if matches!(
        path.as_ref(),
        "/usr/include" | "/usr/local/include" | "/opt/homebrew/include" | "/include"
    ) {
        return true;
    }
    // Debian/Ubuntu multiarch root, e.g. `/usr/include/x86_64-linux-gnu`.
    dir.parent() == Some(Path::new("/usr/include"))
        && dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.ends_with("-gnu") || name.ends_with("-musl") || name.contains("-linux-")
            })
}

/// Names of object-like macros that are pure annotations: ones that expand to
/// nothing (zlib's `FAR`, `ZEXPORT`) or to a GCC `__attribute__((…))` (FFmpeg's
/// `av_const`, nettle's `_NETTLE_ATTRIBUTE_PURE`). libclang consumes these as
/// attributes, but the *source* spelling still carries the macro name, so they
/// must be dropped when a type is reconstructed from source tokens, or they leak
/// into the cdef (`void FAR *`, `int64_t av_const av_gcd(...)`). Gathered from the
/// whole TU, since they usually live in an #included header.
fn gather_empty_macros(translation_unit: &Entity<'_>) -> BTreeSet<String> {
    let mut empty = BTreeSet::new();
    for entity in translation_unit.get_children() {
        if entity.get_kind() != EntityKind::MacroDefinition
            || entity.is_builtin_macro()
            || entity.is_function_like_macro()
        {
            continue;
        }
        let (Some(name), Some(range)) = (entity.get_name(), entity.get_range()) else {
            continue;
        };
        let body: Vec<String> = range
            .tokenize()
            .iter()
            .map(|token| token.get_spelling())
            .skip_while(|spelling| spelling == &name)
            .collect();
        let is_attribute = body.first().map(String::as_str) == Some("__attribute__");
        // A macro expanding to a lone cv/restrict qualifier (notcurses's
        // `#define RESTRICT restrict`) is droppable for cdef purposes: the qualifier
        // is irrelevant to the FFI ABI, and the macro name would otherwise leak into a
        // type as an unknown identifier (`int * RESTRICT y`), failing `FFI::cdef`.
        let is_qualifier = body.len() == 1 && is_type_qualifier_keyword(&body[0]);
        if body.is_empty() || is_attribute || is_qualifier {
            empty.insert(name);
        }
    }
    empty
}

/// A C type-qualifier keyword (irrelevant to the FFI ABI), so a macro expanding to
/// exactly one of these can be stripped from the cdef like an empty macro.
fn is_type_qualifier_keyword(token: &str) -> bool {
    matches!(
        token,
        "const" | "volatile" | "restrict" | "__restrict" | "__restrict__" | "__const"
    )
}

fn collect(
    translation_unit: &Entity<'_>,
    main_header: &Path,
    owned_dirs: &[PathBuf],
    owned_files: &BTreeSet<PathBuf>,
) -> Collected {
    let mut collected = Collected::default();
    let empty_macros = gather_empty_macros(translation_unit);

    for entity in translation_unit.get_children() {
        // Declarations from system / other-library headers are not emitted
        // wholesale, but their *types* are pooled so a kept declaration that
        // references them (e.g. zlib functions using `voidpf` from `zconf.h`)
        // can have the real definition resolved into the cdef.
        if !entity_owned(&entity, main_header, owned_dirs, owned_files) {
            match entity.get_kind() {
                EntityKind::TypedefDecl => collect_pool_typedef(&entity, &mut collected),
                EntityKind::EnumDecl => collect_pool_enum_constants(&entity, &mut collected),
                _ => {}
            }
            continue;
        }

        match entity.get_kind() {
            EntityKind::FunctionDecl => collect_function(&entity, &mut collected, &empty_macros),
            EntityKind::VarDecl => collect_global(&entity, &mut collected),
            EntityKind::StructDecl => collect_struct(&entity, &mut collected),
            EntityKind::UnionDecl => collect_union(&entity, &mut collected),
            EntityKind::EnumDecl => collect_enum(&entity, &mut collected),
            EntityKind::TypedefDecl => collect_typedef(&entity, &mut collected),
            EntityKind::MacroDefinition => collect_macro(&entity, &mut collected),
            _ => {}
        }
    }

    collected.functions.sort();
    collected.functions.dedup();
    collected.typedefs.sort();
    collected.typedefs.dedup();
    collected
}

/// Whether an aggregate body string (`struct X { … }`) declares a field whose
/// identifier collides with a known type name — the signature of a parse corrupted
/// by an undefined annotation macro (glib's `G_GNUC_BEGIN_IGNORE_DEPRECATIONS`
/// around poppler's `GTime mtime;` shifts the declarator so libclang reports the
/// *type* token, e.g. `GString`, as the field name). Emitting `int GString;` where
/// `GString` is a typedef makes PHP FFI reject the whole cdef, so such a struct must
/// stay opaque. Conservatively skips bodies with nested aggregates (anonymous
/// `struct {…}` members), whose `;`-split would misparse.
fn body_has_type_named_field(body: &str, type_names: &BTreeSet<String>) -> bool {
    let (Some(open), Some(close)) = (body.find('{'), body.rfind('}')) else {
        return false;
    };
    let inner = &body[open + 1..close];
    if inner.contains('{') {
        return false;
    }
    inner.split(';').any(|field| {
        // The declared identifier is the last `\w+` token, after dropping an array
        // suffix (`x[4]`) or bitfield width (`x : 3`).
        let field = field.split('[').next().unwrap_or(field);
        let field = field.split(':').next().unwrap_or(field);
        field
            .rsplit(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .find(|token| !token.is_empty())
            .is_some_and(|ident| type_names.contains(ident))
    })
}

/// Record the enum's name (projected onto `int` in the cdef) and each of its
/// enumerators with its evaluated integer value (for `const.php`).
fn collect_enum(entity: &Entity<'_>, collected: &mut Collected) {
    let tag = entity
        .get_name()
        .filter(|name| !is_synthetic_anonymous_name(name));
    if let Some(name) = &tag {
        collected.enums.insert(name.clone());
    }
    let mut cases = Vec::new();
    for constant in entity.get_children() {
        if constant.get_kind() != EntityKind::EnumConstantDecl {
            continue;
        }
        if let (Some(name), Some((signed, _))) =
            (constant.get_name(), constant.get_enum_constant_value())
        {
            collected.enum_constants.push((name.clone(), signed));
            cases.push((name, signed));
        }
    }
    // A named, non-empty enum can become a PHP `enum`. Anonymous enums (just a bag
    // of int constants) and empty ones stay in `const.php` only.
    if let Some(name) = tag
        && !cases.is_empty()
    {
        collected.enum_definitions.push(EnumDef { name, cases });
    }
}

/// Record the enumerators of an enum declared in an #included (non-owned) header,
/// for later prefix-filtered emission in `const.php`. The enum's *name* is not
/// registered (no cdef `typedef int`); only a referencing declaration pulls the
/// type in through the pool.
fn collect_pool_enum_constants(entity: &Entity<'_>, collected: &mut Collected) {
    for constant in entity.get_children() {
        if constant.get_kind() != EntityKind::EnumConstantDecl {
            continue;
        }
        if let (Some(name), Some((signed, _))) =
            (constant.get_name(), constant.get_enum_constant_value())
        {
            collected.pool_enum_constants.push((name, signed));
        }
    }
}

fn is_synthetic_anonymous_name(name: &str) -> bool {
    name.contains("(unnamed") || name.contains("(anonymous")
}

/// Record a `#define`. Object-like macros become constant candidates; the body
/// of a function-like macro is kept for constant-argument expansion. Builtin
/// macros and object-like macros with no replacement (e.g. the empty `#define`s
/// used to neutralise ABI macros) are skipped. The macro's own name is the first
/// token, so everything after it is the parameter list and/or replacement.
fn collect_macro(entity: &Entity<'_>, collected: &mut Collected) {
    if entity.is_builtin_macro() {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    let Some(range) = entity.get_range() else {
        return;
    };
    let rest: Vec<Token> = range
        .tokenize()
        .iter()
        .map(|token| (token.get_kind(), token.get_spelling()))
        .skip_while(|(_, spelling)| spelling == &name)
        .collect();

    if entity.is_function_like_macro() {
        if let Some((params, body)) = split_fn_macro(&rest) {
            collected
                .fn_macros
                .insert(name, RawFnMacro { params, body });
        }
        return;
    }

    if rest.is_empty() {
        return;
    }
    collected.macros.push(RawMacro { name, tokens: rest });
}

/// Split a function-like macro's post-name tokens (`( p1 , p2 ) body…`) into its
/// parameter names and body tokens. Returns `None` if the leading parameter list
/// is malformed.
fn split_fn_macro(tokens: &[Token]) -> Option<(Vec<String>, Vec<Token>)> {
    if tokens.first().map(|(_, spelling)| spelling.as_str()) != Some("(") {
        return None;
    }
    let mut params = Vec::new();
    let mut depth = 0usize;
    for (index, (kind, spelling)) in tokens.iter().enumerate() {
        match spelling.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some((params, tokens[index + 1..].to_vec()));
                }
            }
            _ if depth == 1 && *kind == TokenKind::Identifier => params.push(spelling.clone()),
            _ => {}
        }
    }
    None
}

/// Whether an entity's type (a parameter or a struct/union field) is a function or
/// a pointer to one (`int (*cb)(void *)`, a callback typedef like libconfig's
/// `config_include_fn_t`, or a bare function-type that decays to a pointer). Such a
/// type has no PHP callable mapping and is rendered as an opaque `void *`. Uses the
/// canonical type so a typedef'd callback is seen through.
fn is_function_pointer_entity(argument: &Entity<'_>) -> bool {
    let Some(canonical) = argument.get_type().map(|ty| ty.get_canonical_type()) else {
        return false;
    };
    let is_function = |ty: &clang::Type<'_>| {
        matches!(
            ty.get_kind(),
            TypeKind::FunctionPrototype | TypeKind::FunctionNoPrototype
        )
    };
    // Follow the whole pointer chain: a function pointer (`(*)`) and a pointer to one
    // (`(**)`, gmp's `__gmp_get_memory_functions`) both reconstruct into invalid C
    // (the name dangles outside the declarator), so any pointer chain ending in a
    // function is rendered as an opaque `void *`.
    let mut ty = canonical;
    loop {
        if is_function(&ty) {
            return true;
        }
        match ty.get_pointee_type() {
            Some(pointee) => ty = pointee,
            None => return false,
        }
    }
}

/// The C spelling of a builtin scalar (or `void`), taken from its `TypeKind` so the
/// rendered text never carries a typedef name or libclang display junk. Returns
/// `None` for anything that is not a plain arithmetic/void type (enums, aggregates),
/// which a caller treats as "not cleanly renderable".
fn builtin_scalar_spelling(kind: TypeKind) -> Option<&'static str> {
    Some(match kind {
        TypeKind::Void => "void",
        TypeKind::Bool => "_Bool",
        TypeKind::CharS | TypeKind::CharU => "char",
        TypeKind::SChar => "signed char",
        TypeKind::UChar => "unsigned char",
        TypeKind::Short => "short",
        TypeKind::UShort => "unsigned short",
        TypeKind::Int => "int",
        TypeKind::UInt => "unsigned int",
        TypeKind::Long => "long",
        TypeKind::ULong => "unsigned long",
        TypeKind::LongLong => "long long",
        TypeKind::ULongLong => "unsigned long long",
        TypeKind::Float => "float",
        TypeKind::Double => "double",
        TypeKind::LongDouble => "long double",
        _ => return None,
    })
}

/// Spelling of one component (the return type or an argument) of a callback's C
/// signature.
///
/// A POINTER component keeps its real, spelled type (`const point *`, `const char
/// *`) so the PHP closure receives a typed `\FFI\CData` it can read directly without
/// a cast — the package already emits the struct/typedef, so the type resolves; a
/// type referenced *only* through a callback is backfilled by [`missing_function_types`]
/// so the cdef still loads. It degrades to an opaque `void *` only when the spelling
/// carries syntax the cdef can't place inside a function-pointer declarator: a nested
/// function pointer's parentheses, an array, or an anonymous aggregate with no name.
///
/// A BY-VALUE component must be a plain scalar/`void` (kept at its exact width via
/// [`builtin_scalar_spelling`]); `None` means it is a by-value aggregate/enum with no
/// clean rendering, so the whole parameter falls back to an opaque `void *`.
fn fnptr_component_spelling(ty: &clang::Type<'_>) -> Option<String> {
    if ty.get_canonical_type().get_pointee_type().is_some() {
        let display = ty.get_display_name();
        if display.contains('(')
            || display.contains('[')
            || display.contains("unnamed")
            || display.contains("anonymous")
        {
            return Some("void *".to_owned());
        }
        return Some(display);
    }
    builtin_scalar_spelling(ty.get_canonical_type().get_kind()).map(str::to_owned)
}

/// Render a single-level function-pointer parameter as a real C function-pointer
/// type (`RET (*)(ARGS)`, with the `(*)` left empty for [`declarator`] to weave the
/// parameter name into). Emitting the genuine type — instead of collapsing it to an
/// opaque `void *` — is what lets PHP FFI build a callback trampoline so a PHP
/// `callable` can be passed (verified on PHP 8.5: a closure is accepted for a
/// function-pointer parameter but rejected for a `void *`).
///
/// Returns `None` (caller falls back to `void *`, the prior behavior, no regression)
/// when the type is not exactly one pointer to a function prototype (e.g. gmp's
/// pointer-to-function-pointer) or when any component is a by-value aggregate/enum
/// [`fnptr_component_spelling`] won't normalize.
fn function_pointer_param_type(argument: &Entity<'_>) -> Option<String> {
    let canonical = argument.get_type()?.get_canonical_type();
    if canonical.get_kind() != TypeKind::Pointer {
        return None;
    }
    let function = canonical.get_pointee_type()?;
    if !matches!(
        function.get_kind(),
        TypeKind::FunctionPrototype | TypeKind::FunctionNoPrototype
    ) {
        return None;
    }
    let return_type = fnptr_component_spelling(&function.get_result_type()?)?;
    let mut parts = function
        .get_argument_types()
        .unwrap_or_default()
        .iter()
        .map(fnptr_component_spelling)
        .collect::<Option<Vec<_>>>()?;
    if function.is_variadic() {
        parts.push("...".to_owned());
    }
    let arguments = if parts.is_empty() {
        "void".to_owned()
    } else {
        parts.join(", ")
    };
    Some(format!("{return_type} (*)({arguments})"))
}

/// Rewrite an array parameter declarator to the equivalent pointer (`T name[N]`
/// is `T *name` in C). PHP FFI can't size an array of an incomplete struct
/// (libtiff's `const TIFFFieldInfo arg1[]`), and a fixed byte array param
/// (`char arg3[1024]`) is likewise just a pointer at the ABI.
fn array_param_to_pointer(param: &str) -> String {
    let Some(open) = param.find('[') else {
        return param.to_owned();
    };
    let head = param[..open].trim_end();
    let name_start = head
        .rfind(|ch: char| !is_c_identifier_char(ch))
        .map_or(0, |index| index + 1);
    let (type_part, name) = head.split_at(name_start);
    format!("{}*{name}", type_part)
}

fn collect_function(
    entity: &Entity<'_>,
    collected: &mut Collected,
    empty_macros: &BTreeSet<String>,
) {
    let Some(name) = entity.get_name() else {
        return;
    };
    // A non-externally-linked function has no exported symbol to bind. `static inline`
    // API helpers are common, though, so rather than drop them silently they are
    // surfaced below as a throwing stub method (the API stays complete and calling one
    // gives a clear error). A plain file-scope `static` function is genuinely internal
    // and dropped.
    let exported = entity.get_linkage() == Some(Linkage::External);
    if !exported && !entity.is_inline_function() {
        return;
    }
    let return_type = entity
        .get_result_type()
        .map(|ty| ty.get_display_name())
        .unwrap_or_else(|| "void".to_owned());
    let return_type = source_return_type(entity, &name, empty_macros).unwrap_or(return_type);
    let return_type = fill_missing_pointer_base(&return_type);
    // A function whose *return* type is itself a function pointer (`RET (*)(args)`,
    // openssl's `DSA_meth_get_sign`) can't be woven into a plain declarator without
    // producing an invalid "function returning function" (`RET (*name)(args)(params)`).
    // Render the return as an opaque `void *` — a function pointer is pointer-sized
    // and PHP can't call a returned C callback anyway, mirroring the `void *`
    // treatment of function-pointer parameters and struct fields.
    let return_type = if return_type.contains("(*") {
        "void *".to_owned()
    } else {
        return_type
    };

    let params = entity
        .get_arguments()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let param_name = argument.get_name().unwrap_or_else(|| format!("arg{index}"));
            // A function-pointer parameter (`int (*cb)(void *)`) is rendered as a
            // real C function-pointer type so PHP FFI can build a callback trampoline
            // and the generated wrapper can accept a PHP `callable`. When it can't be
            // reconstructed cleanly (pointer-to-function-pointer, by-value aggregate
            // arguments) it falls back to an opaque `void *`, the prior behavior.
            if is_function_pointer_entity(argument) {
                return match function_pointer_param_type(argument) {
                    Some(pointer_type) => declarator(&pointer_type, &param_name),
                    None => format!("void *{param_name}"),
                };
            }
            let type_name = argument
                .get_type()
                .map(|ty| ty.get_display_name())
                .unwrap_or_default();
            let type_name = source_argument_type(argument, empty_macros).unwrap_or(type_name);
            let type_name = fill_missing_pointer_base(&type_name);
            array_param_to_pointer(&declarator(&type_name, &param_name))
        })
        .collect::<Vec<_>>();

    let mut params = if params.is_empty() {
        "void".to_owned()
    } else {
        params.join(", ")
    };
    // A variadic C function (`printf(const char *, ...)`) is bindable: PHP FFI can
    // call it and the generated method forwards the extra arguments verbatim. Append
    // the C ellipsis so the cdef declaration — and the signature parsed back from it —
    // carry it, instead of dropping the whole declaration.
    if entity.is_variadic() {
        if params == "void" {
            params = "...".to_owned();
        } else {
            params.push_str(", ...");
        }
    }

    let declaration = format!("{}({});", declarator(&return_type, &name), params);
    if exported {
        // Keep only the first declaration of a given name. A header may declare the
        // same function twice (gmp's `__gmpz_size` appears with a named and an unnamed
        // param); the spellings differ so `dedup()` can't merge them, and FFI rejects
        // the redefinition. `insert` is false when the name was already collected.
        if collected.function_names.insert(name.clone()) {
            collected.functions.push(declaration);
        }
    } else if collected.unsupported_names.insert(name.clone()) {
        // `static inline`: a throwing stub, kept out of the cdef (no real symbol).
        collected
            .unsupported_functions
            .push((declaration, "static inline".to_owned()));
    }
}

/// Collect an exported global variable as an `extern <type> <name>;` declaration,
/// so a PHP example can address it through `NativeLibrary::addressOf()`. Only
/// externally-linked globals (a real exported symbol) are kept; `static` file-scope
/// globals and anything whose type can't be spelled cleanly are skipped.
fn collect_global(entity: &Entity<'_>, collected: &mut Collected) {
    if entity.get_linkage() != Some(Linkage::External) {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    let Some(field_type) = entity.get_type() else {
        return;
    };
    if is_function_pointer_entity(entity) {
        return;
    }
    let type_name = field_type.get_display_name();
    if type_name.contains("(unnamed")
        || type_name.contains("(anonymous")
        || type_name.contains('(')
        || type_name.contains('[')
    {
        return;
    }
    collected
        .globals
        .push(format!("extern {};", declarator(&type_name, &name)));
}

/// libclang's error recovery on an unknown type (e.g. `FILE` in a header that
/// forgot to `#include <stdio.h>`) can drop the base type from a declaration's
/// source range, leaving a bare pointer like `*outfile`. Substitute `void` for
/// the missing base so the cdef stays valid C; PHP FFI passes the pointer
/// opaquely either way.
fn fill_missing_pointer_base(type_name: &str) -> String {
    if !type_name.contains('*') {
        return type_name.to_owned();
    }
    let has_base = type_name
        .split(|ch: char| ch == '*' || ch.is_whitespace())
        .any(|token| !matches!(token, "" | "const" | "volatile" | "restrict"));
    if has_base {
        type_name.to_owned()
    } else {
        format!("void {}", type_name.trim_start())
    }
}

fn source_return_type(
    entity: &Entity<'_>,
    function_name: &str,
    empty_macros: &BTreeSet<String>,
) -> Option<String> {
    let tokens = source_tokens(entity)?;
    let name_index = tokens
        .iter()
        .position(|token| token.as_str() == function_name)?;
    let mut type_tokens = tokens[..name_index]
        .iter()
        .filter(|token| !is_function_decl_modifier(token) && !empty_macros.contains(*token))
        .cloned()
        .collect::<Vec<_>>();
    while type_tokens
        .last()
        .is_some_and(|token| is_abi_macro_token(token))
    {
        type_tokens.pop();
    }
    // Drop a leading export/visibility annotation macro (e.g. libxml2's
    // `XMLPUBFUN`) that survives in the source spelling, while keeping at least
    // the real type token.
    while type_tokens.len() > 1 && is_annotation_macro(&type_tokens[0]) {
        type_tokens.remove(0);
    }
    simple_type_from_tokens(&type_tokens)
}

/// An all-uppercase identifier that is not a builtin type name — i.e. an
/// export/calling-convention annotation macro (`XMLPUBFUN`, `DECLSPEC`, …)
/// rather than part of the type.
fn is_annotation_macro(token: &str) -> bool {
    token.len() >= 2
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        && token.chars().any(|ch| ch.is_ascii_uppercase())
        && !builtin_type_names().contains(token)
}

fn source_argument_type(argument: &Entity<'_>, empty_macros: &BTreeSet<String>) -> Option<String> {
    let name = argument.get_name()?;
    let mut tokens = source_tokens(argument)?;
    let name_index = tokens.iter().rposition(|token| token.as_str() == name)?;
    // A trailing array declarator (`T name[]`/`T name[N]`) decays to a pointer in C,
    // but it sits *after* the name and is dropped with it below — so detect it here
    // and add the pointer level. Without this an array parameter whose element is a
    // typedef'd pointer collapses one level (oniguruma's `OnigEncoding encodings[]`
    // becomes `OnigEncoding`, not `OnigEncoding *`).
    let is_array = tokens.get(name_index + 1).is_some_and(|token| token == "[");
    tokens.drain(name_index..);
    // Drop empty annotation macros (e.g. `FAR`) and ABI/calling-convention
    // macros that survived in the source spelling, so they don't leak into the
    // cdef as bogus type tokens.
    tokens.retain(|token| !empty_macros.contains(token) && !is_abi_macro_token(token));
    let type_name = simple_type_from_tokens(&tokens)?;
    Some(if is_array {
        format!("{type_name} *")
    } else {
        type_name
    })
}

fn source_tokens(entity: &Entity<'_>) -> Option<Vec<String>> {
    Some(
        entity
            .get_range()?
            .tokenize()
            .iter()
            .map(|token| token.get_spelling())
            .collect(),
    )
}

fn simple_type_from_tokens(tokens: &[String]) -> Option<String> {
    if tokens.is_empty()
        || tokens
            .iter()
            .any(|token| matches!(token.as_str(), "(" | ")" | "," | ";"))
    {
        return None;
    }
    Some(format_type_tokens(tokens))
}

fn is_function_decl_modifier(token: &str) -> bool {
    matches!(token, "extern" | "static" | "inline") || is_abi_macro_token(token)
}

fn format_type_tokens(tokens: &[String]) -> String {
    let mut out = String::new();
    for token in tokens {
        match token.as_str() {
            "*" => {
                if !out.ends_with(' ') && !out.ends_with('*') && !out.is_empty() {
                    out.push(' ');
                }
                out.push('*');
            }
            "[" => out.push('['),
            "]" => out.push(']'),
            _ if out.ends_with('[') => out.push_str(token),
            _ => {
                // Separate the previous token from this one with a space, unless
                // the buffer is empty or already ends with a space. A trailing `*`
                // still needs the space (`int *` then `name` → `int *name` is wrong
                // here; tokens are space-joined and pointers reattach later).
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(token);
            }
        }
    }
    out
}

fn collect_struct(entity: &Entity<'_>, collected: &mut Collected) {
    if !entity.is_definition() {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    if is_synthetic_anonymous_name(&name) {
        return;
    }

    // If any field cannot be rendered cleanly, leave the struct opaque
    // (forward-declared) instead of emitting an invalid definition.
    let fields: Option<Vec<String>> = entity
        .get_children()
        .into_iter()
        .filter(|child| child.get_kind() == EntityKind::FieldDecl)
        .map(|field| render_union_field(&field, collected))
        .collect();

    if let Some(fields) = fields {
        collected.structs.insert(
            name.clone(),
            format!("struct {name} {{ {} }};", fields.join(" ")),
        );
    }
}

fn collect_union(entity: &Entity<'_>, collected: &mut Collected) {
    if !entity.is_definition() {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    if is_synthetic_anonymous_name(&name) {
        return;
    }

    let fields: Option<Vec<String>> = entity
        .get_children()
        .into_iter()
        .filter(|child| child.get_kind() == EntityKind::FieldDecl)
        .map(|field| render_struct_field(&field))
        .collect();

    if let Some(fields) = fields {
        collected.unions.insert(
            name.clone(),
            format!("union {name} {{ {} }};", fields.join(" ")),
        );
    }
}

fn render_union_field(field: &Entity<'_>, collected: &Collected) -> Option<String> {
    let field_type = field.get_type()?;
    // A function-pointer field — inline `void (*destroy)(…)` or a typedef'd callback
    // (libconfig's `config_include_fn_t include_fn`) — is rendered as an opaque,
    // pointer-sized `void *`. Otherwise the None-gate below would drop the whole
    // struct (a callback typedef is not a builtin value field), keeping it opaque.
    if is_function_pointer_entity(field) {
        let name = field.get_name()?;
        return Some(format!("void *{name};"));
    }
    // An enum field projects to `int` in the cdef (`typedef int <name>;`), so it is a
    // renderable value field even though its typedef name is not a builtin scalar
    // (libconfig's `config_error_t error_type`). Without this it would trip the
    // None-gate below and leave the whole struct opaque.
    let is_enum_field = field_type.get_canonical_type().get_kind() == TypeKind::Enum;
    let display = field_type.get_display_name();
    if !display.contains('*') && !is_enum_field {
        if let Some(declaration) = field_type.get_declaration()
            && matches!(
                declaration.get_kind(),
                EntityKind::StructDecl | EntityKind::UnionDecl
            )
            && declaration.is_anonymous()
        {
            return render_struct_field(field);
        }
        // A by-value member that is a known aggregate (`JSValueUnion u`) is rendered
        // here, and the EMISSION-TIME gate decides safety: `has_incomplete_value_member`
        // (with full collected state) keeps the enclosing aggregate opaque when the
        // member's aggregate is not actually emitted, and `ordered_struct_definitions`
        // emits dependencies first. The safety predicates MUST run at render time, not
        // here — during collection the state is partial (a referenced type may not be
        // collected yet), so `is_safe_union_definition` would answer inconsistently.
        if collected.struct_aliases.contains_key(&display)
            || collected.structs.contains_key(&display)
            || collected.union_aliases.contains_key(&display)
            || collected.unions.contains_key(&display)
        {
            return render_struct_field(field);
        }
        if !is_builtin_value_field_type(&display) {
            // A field whose type is a typedef to a builtin scalar (hiredis's
            // `redisFD fd`, where `redisFD` is `typedef int`) is renderable: emit it
            // as the RESOLVED scalar so it is sizable without the gate having to know
            // the typedef name and without depending on that typedef being emitted.
            // This keeps the enclosing struct from going opaque over a scalar alias.
            if let Some(name) = field.get_name()
                && let Some(scalar) = field_value_scalar_spelling(&field_type)
            {
                return Some(format!("{} {name};", scalar));
            }
            return None;
        }
    }

    render_struct_field(field)
}

/// The resolved builtin-scalar spelling for a by-value field whose canonical type is
/// an arithmetic scalar or an enum (e.g. a typedef `redisFD` → `int`), or `None` for
/// anything else (pointers, records, arrays — handled elsewhere). Lets a struct keep
/// a scalar-typedef field without the conservative opacity gate dropping the whole
/// aggregate over an unrecognised typedef name.
fn field_value_scalar_spelling(field_type: &clang::Type<'_>) -> Option<String> {
    let canonical = field_type.get_canonical_type();
    if canonical.get_pointee_type().is_some() {
        return None;
    }
    if canonical.get_kind() == TypeKind::Enum {
        return Some("int".to_owned());
    }
    builtin_scalar_spelling(canonical.get_kind()).map(str::to_owned)
}

fn is_builtin_value_field_type(display: &str) -> bool {
    let base = display
        .split('[')
        .next()
        .unwrap_or(display)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    matches!(
        base.as_str(),
        "char"
            | "signed char"
            | "unsigned char"
            | "short"
            | "short int"
            | "unsigned short"
            | "unsigned short int"
            | "int"
            | "unsigned"
            | "unsigned int"
            | "long"
            | "long int"
            | "unsigned long"
            | "unsigned long int"
            | "long long"
            | "long long int"
            | "unsigned long long"
            | "unsigned long long int"
            | "float"
            | "double"
            | "long double"
            | "bool"
            | "_Bool"
            | "size_t"
            | "ssize_t"
            | "intptr_t"
            | "uintptr_t"
            | "int8_t"
            | "uint8_t"
            | "int16_t"
            | "uint16_t"
            | "int32_t"
            | "uint32_t"
            | "int64_t"
            | "uint64_t"
            | "Uint8"
            | "Uint16"
            | "Uint32"
            | "Uint64"
            | "Sint8"
            | "Sint16"
            | "Sint32"
            | "Sint64"
    )
}

/// Render one struct field, recursing into anonymous nested unions/structs and
/// weaving the field name into function-pointer types. Returns `None` for a
/// field libclang can only describe by source location (which would produce
/// invalid C), signalling that the whole struct should stay opaque.
fn render_struct_field(field: &Entity<'_>) -> Option<String> {
    let name = field.get_name()?;
    let field_type = field.get_type()?;

    // A function-pointer field (inline `void (*destroy)(…)` or a typedef'd callback)
    // is rendered as an opaque, pointer-sized `void *`: ABI-compatible, the generated
    // PHP can't invoke a struct-stored callback anyway, and it keeps the enclosing
    // struct loadable when the callback's parameters reference a type PHP FFI rejects
    // in a struct context (libmongoc's `_mongoc_stream_t`, libconfig's `config_t`).
    if is_function_pointer_entity(field) {
        return Some(format!("void *{name};"));
    }

    if let Some(declaration) = field_type.get_declaration()
        && matches!(
            declaration.get_kind(),
            EntityKind::UnionDecl | EntityKind::StructDecl
        )
        && declaration.is_anonymous()
    {
        let keyword = if declaration.get_kind() == EntityKind::UnionDecl {
            "union"
        } else {
            "struct"
        };
        let inner: Vec<String> = declaration
            .get_children()
            .into_iter()
            .filter(|child| child.get_kind() == EntityKind::FieldDecl)
            .map(|child| render_struct_field(&child))
            .collect::<Option<_>>()?;
        return Some(format!("{keyword} {{ {} }} {name};", inner.join(" ")));
    }

    let display = field_type.get_display_name();
    if display.contains("(unnamed") || display.contains("(anonymous") {
        return None;
    }
    // Any remaining parenthesised field type is not a plain value (a function-pointer
    // field was already collapsed to `void *` above); leave the struct opaque.
    if display.contains('(') || display.contains(')') {
        return None;
    }
    // A flexible array member (`unsigned char x[]`) has no size; PHP FFI rejects
    // it. Give it one element so the aggregate is sizable (a union is sized by its
    // largest member anyway; such a field is only the last member of a struct).
    let display = display.replace("[]", "[1]");
    Some(format!("{};", declarator(&display, &name)))
}

/// A same-size struct re-expression for a scalar type PHP FFI can't represent,
/// or `None` for ordinary types. `_Complex double`/`float` become a `{re, im}`
/// pair (their C layout), and `__int128`/`unsigned __int128` a `{lo, hi}` pair of
/// 64-bit halves — both stay passable by value, unlike a bare `int` fallback that
/// would silently shrink them.
fn unsupported_scalar_typedef(name: &str, kind: TypeKind, display: &str) -> Option<String> {
    match kind {
        TypeKind::Complex => {
            let element = if display.contains("float") {
                "float"
            } else {
                "double"
            };
            Some(format!(
                "typedef struct {{ {element} re; {element} im; }} {name};"
            ))
        }
        TypeKind::Int128 | TypeKind::UInt128 => Some(format!(
            "typedef struct {{ uint64_t lo; uint64_t hi; }} {name};"
        )),
        _ => None,
    }
}

/// Whether a name is a C fundamental type / keyword that must never be redefined
/// by a typedef. libmongoc's headers yield a spurious `typedef int bool(int *);`
/// that turns the builtin `bool` into a function type, breaking every struct with
/// a `bool` field; PHP FFI already knows these names, so any typedef over one is an
/// artifact to drop.
fn is_c_fundamental_type_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "_Bool"
            | "char"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "void"
            | "signed"
            | "unsigned"
            | "wchar_t"
    )
}

fn collect_typedef(entity: &Entity<'_>, collected: &mut Collected) {
    let Some(name) = entity.get_name() else {
        return;
    };
    if is_synthetic_anonymous_name(&name) || is_c_fundamental_type_name(&name) {
        return;
    }
    let Some(underlying) = entity.get_typedef_underlying_type() else {
        return;
    };

    let canonical = underlying.get_canonical_type();
    let canonical_kind = canonical.get_kind();
    // PHP FFI rejects `_Complex` and 128-bit integers. Re-express them as a
    // same-size struct so the typedef parses and the type stays passable by value
    // (a complex is layout-compatible with two reals; an __int128 with two u64s).
    if let Some(decl) =
        unsupported_scalar_typedef(&name, canonical_kind, &canonical.get_display_name())
    {
        collected.typedef_names.insert(name);
        collected.typedefs.push(decl);
        return;
    }

    match canonical_kind {
        // Enums are projected onto `int`, which is what FFI cares about.
        TypeKind::Enum => {
            collected.enums.insert(name);
        }
        TypeKind::Record => {
            // `typedef struct { … } jpeg_component_info;` has no tag of its own;
            // libclang names it `(unnamed struct at …)`, which is not valid C.
            // Use the typedef's own name as the (opaque) tag instead.
            if let Some(tag) = underlying
                .get_display_name()
                .strip_prefix("struct ")
                .map(str::to_owned)
            {
                let tag = if is_synthetic_anonymous_name(&tag) {
                    name.clone()
                } else {
                    tag
                };
                collected.struct_aliases.insert(name, tag);
            } else if let Some(tag) = underlying
                .get_display_name()
                .strip_prefix("union ")
                .map(str::to_owned)
            {
                let tag = if is_synthetic_anonymous_name(&tag) {
                    name.clone()
                } else {
                    tag
                };
                collected.union_aliases.insert(name, tag);
            }
        }
        _ => {
            collected.typedef_names.insert(name.clone());
            collected
                .typedefs
                .push(typedef_declaration(&name, &underlying.get_display_name()));
        }
    }
}

/// Pool a typedef from an included header for later closure resolution. Stores
/// the surface underlying type (e.g. `void *` for `voidpf`), or a struct/union
/// tag alias, or an enum projected onto `int`. Synthetic/anonymous names are
/// skipped.
fn collect_pool_typedef(entity: &Entity<'_>, collected: &mut Collected) {
    let Some(name) = entity.get_name() else {
        return;
    };
    if is_synthetic_anonymous_name(&name) || is_c_fundamental_type_name(&name) {
        return;
    }
    let Some(underlying) = entity.get_typedef_underlying_type() else {
        return;
    };
    match underlying.get_canonical_type().get_kind() {
        TypeKind::Enum => {
            collected.pool_enums.insert(name);
        }
        TypeKind::Record => {
            // An anonymous record typedef carries an `(unnamed …)` tag; fall back
            // to the typedef's own name so the resolved alias is valid C.
            let display = underlying.get_display_name();
            if let Some(tag) = display.strip_prefix("struct ") {
                let tag = if is_synthetic_anonymous_name(tag) {
                    name.clone()
                } else {
                    tag.to_owned()
                };
                collected.pool_struct_aliases.insert(name, tag);
            } else if let Some(tag) = display.strip_prefix("union ") {
                let tag = if is_synthetic_anonymous_name(tag) {
                    name.clone()
                } else {
                    tag.to_owned()
                };
                collected.pool_union_aliases.insert(name, tag);
            }
        }
        _ => {
            collected
                .pool_typedefs
                .entry(name)
                .or_insert_with(|| underlying.get_display_name());
        }
    }
}

/// Combine a type and an identifier into a C declarator, attaching pointers and
/// array extents to the identifier the way C syntax requires.
fn declarator(type_name: &str, identifier: &str) -> String {
    let type_name = type_name.trim();
    // Function-pointer type (`void (*)(int)`): the identifier belongs inside the
    // `(*)`, giving `void (*name)(int)` — appending it after would be invalid C.
    if type_name.contains("(*)") {
        return type_name.replacen("(*)", &format!("(*{identifier})"), 1);
    }
    if let Some(open) = type_name.find('[') {
        let (base, array) = type_name.split_at(open);
        return format!("{} {identifier}{}", base.trim(), array.trim());
    }
    if type_name.ends_with('*') {
        format!("{type_name}{identifier}")
    } else {
        format!("{type_name} {identifier}")
    }
}

fn typedef_declaration(name: &str, underlying: &str) -> String {
    if let Some(open) = underlying.find('(') {
        // Distinguish a function-*pointer* typedef (`ret (*)(args)`, where the
        // first `(` opens the pointer declarator) from a function-*type* typedef
        // (`ret name(args)`). The discriminator is whether the first `(` is
        // immediately followed by `*`. A bare `underlying.contains("(*)")` test is
        // wrong when the typedef is a function *type* whose parameters are
        // themselves function pointers: its display carries a `(*)` belonging to a
        // *parameter*, and naively weaving the name into that first `(*)` mangles
        // it into `typedef int (const int *, int (*name)(…), …);` (openssl's
        // `OSSL_FUNC_provider_register_child_cb_fn`).
        if underlying[open + 1..].trim_start().starts_with('*') {
            return format!(
                "typedef {};",
                underlying.replacen("(*)", &format!("(*{name})"), 1)
            );
        }
        // A function-type typedef (`typedef int handler(void *, size_t);`): the
        // name belongs between the return type and the parameter list, not after
        // the whole thing (which would yield `typedef int (args) name;`). Any
        // function-pointer *parameters* in the list are left untouched.
        let (return_type, params) = underlying.split_at(open);
        return format!("typedef {} {name}{};", return_type.trim(), params);
    }
    format!("typedef {};", declarator(underlying, name))
}

/// The prefix-matching constants worth surfacing as PHP `const`s, in emit order:
/// enum constants first (exact integer values), then object-like `#define`s.
///
/// A macro is kept only if its replacement is a constant expression PHP can
/// evaluate; a single forward pass lets a later macro reference an
/// already-emitted constant by name — including the enum constants seeded ahead
/// of it (e.g. `#define FOO (SOME_ENUM | 1)`). Anything untranslatable (casts,
/// char literals, calls, unknown names) is silently dropped, and an enum value
/// wins over any later macro of the same name.
fn header_constants(
    collected: &Collected,
    prefix: &str,
    definitions: &[crate::model::manifest::ResolvedDefinition],
) -> Vec<Constant> {
    let needle = prefix.to_ascii_lowercase();
    // Every evaluated constant by name (including non-prefix ones), so a macro that
    // references another constant resolves to its *value*. Folding the reference to
    // a literal is what lets the wrapped form work: a wrapped constant is a
    // `\Pnlx\Types\*` object that could not take part in PHP arithmetic.
    let mut env: BTreeMap<String, ConstValue> = BTreeMap::new();
    let mut emitted = BTreeSet::new();
    let mut constants = Vec::new();

    // Seed the resolved `require_definitions` so a macro that references one (pcre2's
    // width-keyed expressions) resolves, and emit each as a typed constant.
    for definition in definitions {
        if let Some(value) = definition_const_value(definition) {
            env.entry(definition.name.clone()).or_insert(value);
        }
    }

    // Enumerators (owned and #included) are plain `int`s. Seed every one into the
    // environment first so any macro can reference any enumerator regardless of
    // source order.
    for (name, value) in collected
        .enum_constants
        .iter()
        .chain(&collected.pool_enum_constants)
    {
        env.entry(name.clone())
            .or_insert(ConstValue::Int(*value as i128, IntKind::Int));
    }

    // The resolved definitions are emitted unconditionally (not prefix-filtered): the
    // user asked for them, and the name (e.g. `PCRE2_CODE_UNIT_WIDTH`) need not match.
    for definition in definitions {
        if let Some(value) = definition_const_value(definition)
            && emitted.insert(definition.name.clone())
        {
            let (wrapped, scalar) = render_const_value(&value);
            constants.push(Constant {
                name: definition.name.clone(),
                wrapped,
                scalar,
            });
        }
    }

    let mut emit = |name: &str, value: &ConstValue, constants: &mut Vec<Constant>| {
        if name.to_ascii_lowercase().contains(&needle) && emitted.insert(name.to_owned()) {
            let (wrapped, scalar) = render_const_value(value);
            constants.push(Constant {
                name: name.to_owned(),
                wrapped,
                scalar,
            });
        }
    };

    // Owned enumerators, then #included ones (the owned win on a name clash).
    for (name, value) in &collected.enum_constants {
        emit(
            name,
            &ConstValue::Int(*value as i128, IntKind::Int),
            &mut constants,
        );
    }
    for (name, value) in &collected.pool_enum_constants {
        emit(
            name,
            &ConstValue::Int(*value as i128, IntKind::Int),
            &mut constants,
        );
    }

    // Object-like macros in source order: evaluate against the environment built so
    // far, record the value for later references, and emit the prefix-matching ones.
    for macro_def in &collected.macros {
        let Some(value) = eval_const(&macro_def.tokens, &env, &collected.fn_macros, 0) else {
            continue;
        };
        emit(&macro_def.name, &value, &mut constants);
        env.entry(macro_def.name.clone()).or_insert(value);
    }

    constants
}

/// Bounds nested function-like-macro expansions.
const MAX_MACRO_EXPANSION_DEPTH: usize = 16;

/// Byte-pointer base types PHP FFI will NOT coerce a PHP string into, even though
/// they are the same 1-byte pointer as `char *` at the ABI and the generated
/// wrapper already types a single-level pointer to them as `string`. PHP FFI only
/// passes a string for `char *` (and `void *`), so a parameter of one of these is
/// rewritten to `char *` in the cdef. `char`/`const char` already work and are not
/// listed.
const NON_CHAR_BYTE_POINTER_BASES: &[&str] = &["unsigned char", "signed char", "uint8_t", "int8_t"];

/// Rewrite single-level pointer-to-byte function *parameters* (`const uint8_t *`,
/// `unsigned char *`, oniguruma's `const OnigUChar *`, pcre2's `PCRE2_SPTR8`) to
/// `char *` so PHP FFI accepts a PHP string for them — directly or through a
/// typedef. The return type is left untouched (a string return is read with
/// `FFI::string`, which works on any byte pointer), and so are pointer-to-pointer,
/// arrays, and function-pointer params. A declaration whose parameters are all
/// unchanged is returned byte-for-byte; only ones with a real rewrite are
/// re-joined.
fn rewrite_byte_pointer_params(decl: &str, typedefs: &BTreeMap<String, String>) -> String {
    let (Some(open), Some(close)) = (decl.find('('), decl.rfind(')')) else {
        return decl.to_owned();
    };
    if close <= open + 1 {
        return decl.to_owned();
    }
    let head = &decl[..=open];
    let params = &decl[open + 1..close];
    let tail = &decl[close..];
    let originals = split_top_level_commas(params);
    let rewritten: Vec<String> = originals
        .iter()
        .map(|param| rewrite_byte_pointer_param(param, typedefs))
        .collect();
    if originals
        .iter()
        .zip(&rewritten)
        .all(|(original, new)| original.trim() == new.as_str())
    {
        return decl.to_owned();
    }
    format!("{head}{}{tail}", rewritten.join(", "))
}

/// Rewrite one parameter to `char *` when it is (or resolves through a typedef to)
/// a single-level pointer to a non-`char` byte type PHP FFI won't accept a string
/// for; otherwise return it trimmed-but-unchanged.
fn rewrite_byte_pointer_param(param: &str, typedefs: &BTreeMap<String, String>) -> String {
    let trimmed = param.trim();
    if trimmed.contains('[') || trimmed.contains('(') {
        return trimmed.to_owned();
    }
    match trimmed.matches('*').count() {
        // `<quals> <byte> *name`: the element (builtin or a byte typedef like
        // `OnigUChar`) resolves to a non-char byte scalar.
        1 => {
            let star = trimmed.find('*').unwrap();
            let (quals, base) = split_type_qualifiers(&trimmed[..star]);
            if !resolves_to_non_char_byte_scalar(&base, typedefs, 0) {
                return trimmed.to_owned();
            }
            let name = trimmed[star..].trim_start_matches('*').trim();
            let prefix = if quals.is_empty() {
                String::new()
            } else {
                format!("{quals} ")
            };
            format!("{prefix}char *{name}")
        }
        // `<typedef> name`: the typedef itself is a single-level byte pointer
        // (pcre2 `PCRE2_SPTR8` → `const unsigned char *`).
        0 => {
            let Some((type_name, name)) = split_declaration_name(trimmed) else {
                return trimmed.to_owned();
            };
            let (quals, base) = split_type_qualifiers(&type_name);
            match resolves_to_non_char_byte_pointer(&base, typedefs, 0) {
                Some(pointee_const) => {
                    let prefix = if quals.contains("const") || pointee_const {
                        "const "
                    } else {
                        ""
                    };
                    format!("{prefix}char *{name}")
                }
                None => trimmed.to_owned(),
            }
        }
        _ => trimmed.to_owned(),
    }
}

/// Split a type prefix into its `const`/`volatile`/`restrict` qualifiers and the
/// remaining base type tokens.
fn split_type_qualifiers(prefix: &str) -> (String, String) {
    let mut quals = Vec::new();
    let mut base = Vec::new();
    for token in prefix.split_whitespace() {
        if matches!(token, "const" | "volatile" | "restrict") {
            quals.push(token);
        } else {
            base.push(token);
        }
    }
    (quals.join(" "), base.join(" "))
}

/// Whether a type name is, or resolves through single-level scalar typedefs to, a
/// non-`char` byte scalar (`unsigned char`/`signed char`/`uint8_t`/`int8_t`).
/// `char` is excluded — a `char *` already accepts a PHP string.
fn resolves_to_non_char_byte_scalar(
    type_name: &str,
    typedefs: &BTreeMap<String, String>,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }
    let (_, base) = split_type_qualifiers(type_name);
    if NON_CHAR_BYTE_POINTER_BASES.contains(&base.as_str()) {
        return true;
    }
    if base.chars().all(is_c_identifier_char)
        && let Some(underlying) = typedefs.get(&base)
        && !underlying.contains('*')
        && !underlying.contains('[')
        && !underlying.contains('(')
    {
        return resolves_to_non_char_byte_scalar(underlying, typedefs, depth + 1);
    }
    false
}

/// Whether a typedef name resolves to a single-level pointer to a non-`char` byte
/// type. Returns `Some(pointee_is_const)` so the rewrite can keep `const`.
fn resolves_to_non_char_byte_pointer(
    type_name: &str,
    typedefs: &BTreeMap<String, String>,
    depth: usize,
) -> Option<bool> {
    if depth > 16 {
        return None;
    }
    let underlying = typedefs.get(type_name.trim())?;
    match underlying.matches('*').count() {
        1 => {
            let element = underlying.replace('*', " ");
            resolves_to_non_char_byte_scalar(&element, typedefs, depth + 1)
                .then_some(underlying.contains("const"))
        }
        // A plain alias to another typedef (`PCRE2_SPTR` → `PCRE2_SPTR8`).
        0 if underlying
            .chars()
            .all(|ch| is_c_identifier_char(ch) || ch == ' ') =>
        {
            resolves_to_non_char_byte_pointer(underlying.trim(), typedefs, depth + 1)
        }
        _ => None,
    }
}

/// Map of typedef name -> underlying type display for simple (non-function,
/// non-array) typedefs, from the package's own typedefs and the #included pool, so
/// a byte-pointer parameter hidden behind a typedef can be resolved.
fn simple_typedef_map(collected: &Collected) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for decl in &collected.typedefs {
        let Some(inner) = decl
            .strip_prefix("typedef ")
            .map(|rest| rest.trim_end_matches(';').trim())
        else {
            continue;
        };
        if inner.contains('(') || inner.contains('[') {
            continue;
        }
        if let Some((underlying, name)) = split_declaration_name(inner) {
            map.entry(name).or_insert(underlying);
        }
    }
    for (name, underlying) in &collected.pool_typedefs {
        map.entry(name.clone())
            .or_insert_with(|| underlying.clone());
    }
    map
}

/// Split a parameter list on commas at the top paren/bracket nesting level, so a
/// comma inside a function-pointer parameter's own argument list is not a split
/// point.
fn split_top_level_commas(params: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&params[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&params[start..]);
    parts
}

/// A scratch directory for the temporary headers handed to libclang.
struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("pnl-cdef-{}", std::process::id()));
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    fn write(&self, name: &str, contents: &str) -> Result<PathBuf> {
        let path = self.root.join(name);
        std::fs::write(&path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{HeaderAdapterOptions, HeaderArtifacts, cdef_from_header, owned_package_dirs};

    #[test]
    fn body_has_type_named_field_flags_type_named_fields() {
        use super::body_has_type_named_field;
        use std::collections::BTreeSet;
        let types: BTreeSet<String> = ["GString", "GTime"].iter().map(|s| s.to_string()).collect();
        // A field named after a type (corrupted parse) is flagged.
        assert!(body_has_type_named_field(
            "struct _PopplerAttachment { int parent; gchar *name; int GTime; int ctime; int GString; };",
            &types,
        ));
        // A clean struct (no field name collides with a type) is not.
        assert!(!body_has_type_named_field(
            "struct Clean { int a; double b; char *name; unsigned long size; };",
            &types,
        ));
        // Nested anonymous aggregates are conservatively skipped (not flagged).
        assert!(!body_has_type_named_field(
            "struct Nested { struct { int x; } GString; };",
            &types,
        ));
    }

    #[test]
    #[cfg(unix)]
    fn owned_package_dirs_resolves_symlinked_include_roots() {
        // Homebrew exposes a package's headers through a symlinked include dir
        // (`/opt/homebrew/include/<pkg>` -> `…/Cellar/<pkg>/<ver>/include/<pkg>`),
        // but libclang reports the real Cellar path for `#include`d sub-headers.
        // `owned_package_dirs` must surface the resolved real dir too, or those
        // sub-header declarations are dropped as "not owned".
        let tmp = tempfile::tempdir().expect("tempdir");
        let real = tmp.path().join("Cellar/pkg/1.0/include/pkg");
        std::fs::create_dir_all(&real).expect("mkdir real");
        let link = tmp.path().join("link-pkg");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let dirs = owned_package_dirs(&[link.join("pkg.h")]);
        let canonical_real = std::fs::canonicalize(&real).expect("canonicalize");
        assert!(
            dirs.contains(&link) && dirs.contains(&canonical_real),
            "expected both the symlink dir and its resolved real dir, got {dirs:?}"
        );
    }

    fn artifacts(header: &str) -> HeaderArtifacts {
        cdef_from_header(
            header,
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .expect("libclang must be available to run header_adapter tests")
    }

    fn cdef(header: &str) -> String {
        artifacts(header).cdef
    }

    #[test]
    fn recovers_symbol_version_rename_alias() {
        // The ICU shape: a public name is `#define`d to a multi-level rename macro
        // that token-pastes a version suffix. The preprocessor renames the export to
        // `ex_hello_9`; the alias recovers the public `ex_hello -> ex_hello_9` map.
        let artifacts = artifacts(
            "#define EX_VERSION_SUFFIX _9\n\
             #define EX_PASTE2(x, y) x ## y\n\
             #define EX_PASTE(x, y) EX_PASTE2(x, y)\n\
             #define EX_RENAME(x) EX_PASTE(x, EX_VERSION_SUFFIX)\n\
             #define ex_hello EX_RENAME(ex_hello)\n\
             int ex_hello(int code);\n",
        );
        assert!(
            artifacts.cdef.contains("int ex_hello_9(int code);"),
            "the cdef should declare the versioned export: {}",
            artifacts.cdef
        );
        assert!(
            artifacts
                .symbol_aliases
                .contains(&("ex_hello".to_owned(), "ex_hello_9".to_owned())),
            "expected ex_hello -> ex_hello_9, got {:?}",
            artifacts.symbol_aliases
        );
    }

    const HEADER: &str = r#"
        /* example device library */
        #define EX_FLAG_A 0x01
        #define EX_FLAG_B (1 << 1)
        #define EX_FLAGS (EX_FLAG_A | EX_FLAG_B)
        #define EX_NAME "example"
        #define EX_TITLE "lib " EX_NAME " v"
        #define EX_RATIO 1.5f
        #define EX_MAX(a, b) ((a) > (b) ? (a) : (b))
        #define EX_POS_MASK 0x2FFF0000
        #define EX_POS_DISPLAY(X) (EX_POS_MASK | (X))
        #define EX_POS_CENTERED EX_POS_DISPLAY(0)
        #define EX_DOUBLE(N) ex_add(N, N)
        #define EX_TAKE_ADDR(N) ex_add(&(N), N)
        #define EX_VERSION_STR(MJR, MNR) MJR "." MNR
        #define EX_EMPTY_CONCAT() ()
        #define EX_BAD_VERSION 1.2.3
        #define EX_MIX(X, Y) ex_add(1, ex_add(X, Y))
        #define EX_BADCALL(Z) ex_unknown(Z)
        typedef enum ex_color { EX_RED = 0, EX_GREEN, EX_BLUE } ex_color;
        typedef struct ex_point { int x; int y; } ex_point;
        typedef void (*ex_callback)(int code, void *user);
        typedef uint32_t ex_flags;
        static inline int ex_helper(int a) { return a + 1; }
        extern DECLSPEC int EXCALL ex_add(int left, int right);
        DECLSPEC const char * EXCALL ex_version(void);
        int ex_printf(const char *fmt, ...);
        void ex_set_callback(ex_callback cb, void *user);
    "#;

    #[test]
    fn keeps_externally_linked_prefixed_functions() {
        let cdef = cdef(HEADER);
        assert!(cdef.contains("int ex_add(int left, int right);"), "{cdef}");
        assert!(cdef.contains("const char *ex_version(void);"), "{cdef}");
        assert!(cdef.contains("void ex_set_callback("), "{cdef}");
    }

    #[test]
    fn renders_function_pointer_parameters_as_real_callbacks() {
        // A function-pointer parameter is emitted as a genuine C function-pointer
        // type (not an opaque `void *`) so PHP FFI can build a callback trampoline,
        // and each component keeps its real, spelled type so the closure receives a
        // typed CData.
        let inline_cb = cdef(
            "int ex_walk(int (*visit)(int item));\n\
             void ex_sort(int *base, int n, int (*cmp)(const void *a, const void *b));\n",
        );
        assert!(
            inline_cb.contains("int ex_walk(int (*visit)(int));"),
            "inline scalar callback not rendered: {inline_cb}"
        );
        assert!(
            inline_cb.contains("int (*cmp)(const void *, const void *)"),
            "callback pointer args should keep their real spelling: {inline_cb}"
        );
        // A callback reached through a typedef is expanded the same way.
        let typedef_cb = cdef(
            "typedef void (*ex_cb)(int code, void *user);\n\
             void ex_on(ex_cb cb, void *user);\n",
        );
        assert!(
            typedef_cb.contains("void ex_on(void (*cb)(int, void *), void *user);"),
            "typedef'd callback not expanded: {typedef_cb}"
        );
        // A callback argument of a package-defined struct keeps the real struct type
        // (the closure gets a typed CData), and the struct definition is present.
        let struct_cb = cdef(
            "typedef struct ex_point { int x; int y; } ex_point;\n\
             int ex_each(int (*cb)(const ex_point *pt));\n",
        );
        assert!(
            struct_cb.contains("int (*cb)(const struct ex_point *)"),
            "defined struct callback arg should keep its type: {struct_cb}"
        );
        // A pointer-to-function-pointer can't be a PHP callable, so it falls back to
        // the opaque `void *` rendering (no regression).
        let nested_cb = cdef("void ex_slot(void (**slot)(int));\n");
        assert!(
            nested_cb.contains("void ex_slot(void *slot);"),
            "pointer-to-function-pointer should stay opaque: {nested_cb}"
        );
    }

    #[test]
    fn extracts_callback_component_types_for_backfill() {
        use super::render::callback_component_types;
        // The return type and each argument type inside a function-pointer parameter
        // are surfaced so a type reached only through a callback can be backfilled.
        assert_eq!(
            callback_component_types(
                "int ex_each(int value, struct ex_node *(*cb)(const ex_point *, int));"
            ),
            vec![
                "struct ex_node *".to_owned(),
                "const ex_point *".to_owned(),
                "int".to_owned(),
            ]
        );
        // An ordinary (non-callback) parameter contributes no components, and a
        // `void` argument list is skipped.
        assert!(callback_component_types("int ex_add(int a, int b);").is_empty());
        assert_eq!(
            callback_component_types("void ex_on(void (*cb)(void));"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn empty_prefix_header_drops_unexported_functions() {
        use super::filter_verbatim_to_exports;
        let header = "typedef unsigned long size_t;\n\
struct tm { int tm_sec; };\n\
int printf(const char *fmt, ...);\n\
int atexit(void *func);\n\
void qsort(void *base, size_t n, size_t sz, int (*cmp)(const void *, const void *));\n\
void *malloc(size_t n);\n";
        let exported: std::collections::BTreeSet<String> = ["printf", "malloc", "qsort"]
            .into_iter()
            .map(String::from)
            .collect();
        let out = filter_verbatim_to_exports(header, &exported);
        // Exported functions kept; the unexported one (libc_nonshared `atexit`) dropped.
        assert!(out.contains("int printf(const char *fmt, ...);"), "{out}");
        assert!(out.contains("void *malloc(size_t n);"), "{out}");
        assert!(out.contains("void qsort("), "fnptr-param fn kept: {out}");
        assert!(
            !out.contains("atexit"),
            "unexported atexit should be dropped: {out}"
        );
        // Non-function lines pass through verbatim.
        assert!(out.contains("typedef unsigned long size_t;"), "{out}");
        assert!(out.contains("struct tm { int tm_sec; };"), "{out}");
    }

    #[test]
    fn drops_inline_functions_but_keeps_variadic() {
        let cdef = cdef(HEADER);
        // `static inline` has no exported symbol, so it stays out of the cdef.
        assert!(!cdef.contains("ex_helper"), "inline kept: {cdef}");
        // A variadic function is bindable through PHP FFI; keep it with the ellipsis
        // so the generated method can forward the extra arguments.
        assert!(
            cdef.contains("int ex_printf(const char *fmt, ...);"),
            "variadic dropped: {cdef}"
        );
    }

    #[test]
    fn applies_require_definitions_as_defines_and_constants() {
        use crate::model::manifest::{DefinitionType, ResolvedDefinition};
        // A declaration gated on the definition is only collected when the `-D`
        // reaches libclang, so its presence proves the define took effect.
        let header = "#if EX_WIDTH == 8\nint ex_width_fn(int x);\n#endif\nint ex_base(void);\n";
        let artifacts = cdef_from_header(
            header,
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                definitions: vec![ResolvedDefinition {
                    name: "EX_WIDTH".to_owned(),
                    value: "8".to_owned(),
                    definition_type: DefinitionType::Int,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            artifacts.cdef.contains("int ex_width_fn(int x);"),
            "width-gated decl missing (define not applied): {}",
            artifacts.cdef
        );
        // The resolved definition is also emitted as a typed generated constant.
        assert!(
            artifacts
                .constants
                .iter()
                .any(|constant| constant.name == "EX_WIDTH"
                    && constant.wrapped == "new \\Pnlx\\Types\\Int_(8)"),
            "{:?}",
            artifacts.constants
        );
    }

    #[test]
    fn macro_symbol_alias_resolves_through_a_require_definition() {
        use crate::model::manifest::{DefinitionType, ResolvedDefinition};
        // The pcre2 pattern in miniature: the friendly name `ex_open` pastes a width
        // that comes from a `-D` definition (`EX_WIDTH`), reaching the real
        // width-suffixed export `ex_open_8`. The alias must be recovered even though
        // the `-D` value is invisible to libclang's MacroDefinition cursors.
        let header = "#define EX_GLUE2(a, b) a ## b\n\
             #define EX_GLUE(a, b) EX_GLUE2(a, b)\n\
             #define EX_SUFFIX(a) EX_GLUE(a, EX_WIDTH)\n\
             #define ex_open EX_SUFFIX(ex_open_)\n\
             int ex_open_8(int x);\n";
        let artifacts = cdef_from_header(
            header,
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                definitions: vec![ResolvedDefinition {
                    name: "EX_WIDTH".to_owned(),
                    value: "8".to_owned(),
                    definition_type: DefinitionType::Int,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            artifacts
                .symbol_aliases
                .contains(&("ex_open".to_owned(), "ex_open_8".to_owned())),
            "{:?}",
            artifacts.symbol_aliases
        );
    }

    #[test]
    fn surfaces_static_inline_as_an_unsupported_stub() {
        // A `static inline` function is not bound (no symbol) but is not dropped: it
        // is surfaced as a throwing stub method with the `static inline` reason and a
        // faithful declaration (so its signature/types render).
        let unsupported = artifacts(HEADER).unsupported_functions;
        let helper = unsupported
            .iter()
            .find(|function| function.declaration.contains("ex_helper"))
            .expect("ex_helper should be an unsupported stub");
        assert_eq!(helper.reason, "static inline");
        assert_eq!(helper.declaration, "int ex_helper(int a);");
    }

    #[test]
    fn neutralizes_abi_macros_without_losing_declarations() {
        let cdef = cdef(HEADER);
        assert!(!cdef.contains("DECLSPEC"), "{cdef}");
        assert!(!cdef.contains("EXCALL"), "{cdef}");
    }

    #[test]
    fn does_not_neutralize_header_defined_guard_or_value_macros() {
        // The all-caps include guard `EX_H` and the value macro `EX_API_VERSION`
        // look like ABI macros, but stubbing the guard would mark the header as
        // already-included and drop every declaration (the libusb failure mode).
        const GUARDED: &str = r#"
            #ifndef EX_H
            #define EX_H
            #define EX_API_VERSION 0x010203
            #if EX_API_VERSION >= 0x010000
            int EXCALL ex_supported(void);
            #endif
            #endif
        "#;
        let cdef = cdef(GUARDED);
        assert!(cdef.contains("int ex_supported(void);"), "{cdef}");
    }

    #[test]
    fn projects_enums_onto_int_and_preserves_constants() {
        let cdef = cdef(HEADER);
        assert!(cdef.contains("typedef int ex_color;"), "{cdef}");
        assert!(!cdef.contains("enum ex_color"), "{cdef}");
    }

    #[test]
    fn emits_structs_and_function_pointer_typedefs() {
        let cdef = cdef(HEADER);
        assert!(
            cdef.contains("struct ex_point { int x; int y; };"),
            "{cdef}"
        );
        assert!(cdef.contains("typedef struct ex_point ex_point;"), "{cdef}");
        assert!(
            cdef.contains("typedef void (*ex_callback)(int, void *);"),
            "{cdef}"
        );
    }

    #[test]
    fn emits_opaque_struct_typedefs() {
        let cdef = cdef_from_header(
            "typedef struct ex_context ex_context;\n\
             int ex_open(ex_context **ctx);\n\
             void ex_close(ex_context *ctx);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(cdef.contains("struct ex_context;"), "{cdef}");
        assert!(
            cdef.contains("typedef struct ex_context ex_context;"),
            "{cdef}"
        );
        assert!(cdef.contains("int ex_open(ex_context **ctx);"), "{cdef}");
    }

    #[test]
    fn preserves_late_opaque_struct_typedef_parameters() {
        let cdef = cdef_from_header(
            "int ex_use(ex_context *ctx);\n\
             typedef struct ex_context ex_context;",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(
            cdef.contains("typedef struct ex_context ex_context;"),
            "{cdef}"
        );
        assert!(cdef.contains("int ex_use(ex_context *ctx);"), "{cdef}");
        assert!(!cdef.contains("int ex_use(int *ctx);"), "{cdef}");
    }

    #[test]
    fn preserves_late_opaque_struct_typedef_parameters_with_abi_macros() {
        let cdef = cdef_from_header(
            "extern DECLSPEC ex_context * EXCALL ex_create(void);\n\
             extern DECLSPEC int EXCALL ex_use(ex_context *ctx);\n\
             typedef struct ex_context ex_context;",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(cdef.contains("ex_context *ex_create(void);"), "{cdef}");
        assert!(cdef.contains("int ex_use(ex_context *ctx);"), "{cdef}");
        assert!(!cdef.contains("int ex_use(int *ctx);"), "{cdef}");
    }

    #[test]
    fn detects_sdl_style_aggregate_declarations() {
        let header = "typedef struct SDL_Window SDL_Window;\n";
        assert_eq!(
            super::declared_aggregate_kind(header, "SDL_Window"),
            Some("struct")
        );
    }

    #[test]
    fn emits_opaque_union_typedefs() {
        let cdef = cdef_from_header(
            "typedef union ex_event ex_event;\n\
             int ex_poll(ex_event *event);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(cdef.contains("union ex_event;"), "{cdef}");
        assert!(cdef.contains("typedef union ex_event ex_event;"), "{cdef}");
        assert!(cdef.contains("int ex_poll(ex_event *event);"), "{cdef}");
    }

    #[test]
    fn emits_named_union_definitions() {
        let cdef = cdef_from_header(
            "union ex_params { int a; float b; };\n\
             typedef union ex_params ex_params;\n\
             struct ex_mode { ex_params params; };\n\
             typedef struct ex_mode ex_mode;\n\
             int ex_use(ex_mode *mode);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(
            cdef.contains("union ex_params { int a; float b; };"),
            "{cdef}"
        );
        assert!(cdef.contains("int ex_use(ex_mode *mode);"), "{cdef}");
    }

    #[test]
    fn leaves_union_with_named_aggregate_values_opaque() {
        let cdef = cdef_from_header(
            "struct ex_common { int type; };\n\
             typedef struct ex_common ex_common;\n\
             union ex_event { ex_common common; int type; };\n\
             typedef union ex_event ex_event;\n\
             int ex_poll(ex_event *event);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(cdef.contains("union ex_event;"), "{cdef}");
        assert!(!cdef.contains("union ex_event {"), "{cdef}");
        assert!(cdef.contains("int ex_poll(ex_event *event);"), "{cdef}");
    }

    #[test]
    fn orders_struct_definitions_before_value_dependants() {
        let cdef = cdef_from_header(
            "struct ex_z { int value; };\n\
             typedef struct ex_z ex_z;\n\
             struct ex_a { ex_z z; };\n\
             typedef struct ex_a ex_a;\n\
             int ex_use(ex_a *value);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        let z = cdef
            .find("struct ex_z { int value; };")
            .unwrap_or(usize::MAX);
        let a = cdef.find("struct ex_a { ex_z z; };").unwrap_or(usize::MAX);
        assert!(z < a, "{cdef}");
    }

    #[test]
    fn emits_anonymous_union_struct_fields_inline() {
        // Mirrors SDL's SDL_GameControllerButtonBind, whose anonymous union once
        // produced an invalid `union (unnamed union at ...)` field.
        let cdef = cdef_from_header(
            "struct ex_bind { int kind; union { int button; int axis; } value; };",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(
            cdef.contains("struct ex_bind { int kind; union { int button; int axis; } value; };"),
            "{cdef}"
        );
        assert!(!cdef.contains("unnamed"), "{cdef}");
        // The struct definition must not be misread as a function declaration.
        let signatures = crate::codegen::parse_function_signatures(&cdef);
        assert!(
            signatures.iter().all(|sig| sig.name.starts_with("ex_")),
            "unexpected signatures: {:?}",
            signatures.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn drops_synthetic_anonymous_enum_names() {
        let cdef = cdef_from_header(
            "typedef enum { EX_KIND_A = 1, EX_KIND_B = 2 } ex_kind;\n\
             int ex_kind_name(ex_kind kind);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
        .cdef;

        assert!(cdef.contains("typedef int ex_kind;"), "{cdef}");
        assert!(!cdef.contains("unnamed"), "{cdef}");
        assert!(!cdef.contains("anonymous"), "{cdef}");
    }

    #[test]
    fn returns_header_unchanged_without_prefix() {
        let out = cdef_from_header(
            "int whatever(void);",
            &HeaderAdapterOptions {
                symbol_prefix: "  ".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.cdef, "int whatever(void);");
        assert!(out.constants.is_empty());
        assert!(out.macro_functions.is_empty());
    }

    #[test]
    fn extracts_translatable_object_like_macros() {
        let constants = artifacts(HEADER).constants;
        let find = |name: &str| constants.iter().find(|constant| constant.name == name);
        let wrapped = |name: &str| find(name).map(|constant| constant.wrapped.as_str());
        let scalar = |name: &str| find(name).map(|constant| constant.scalar.as_str());

        // Values are evaluated at generation time and wrapped in a typed object:
        // hex/shift expressions are folded, and a reference to an earlier constant is
        // flattened to its value (a wrapped constant can't take part in PHP math).
        assert_eq!(wrapped("EX_FLAG_A"), Some("new \\Pnlx\\Types\\Int_(1)"));
        assert_eq!(wrapped("EX_FLAG_B"), Some("new \\Pnlx\\Types\\Int_(2)"));
        assert_eq!(wrapped("EX_FLAGS"), Some("new \\Pnlx\\Types\\Int_(3)"));
        // `0x2FFF0000` = 805240832; `EX_POS_DISPLAY(0)` folds to the same value.
        assert_eq!(
            wrapped("EX_POS_MASK"),
            Some("new \\Pnlx\\Types\\Int_(805240832)")
        );
        assert_eq!(
            wrapped("EX_POS_CENTERED"),
            Some("new \\Pnlx\\Types\\Int_(805240832)")
        );

        // `1.5f` carries the C `float` type, so it wraps in `Float_` (not `Double`).
        assert_eq!(wrapped("EX_RATIO"), Some("new \\Pnlx\\Types\\Float_(1.5)"));

        // Strings wrap in `String_`; C-adjacent literals/macros are joined with `.`.
        assert_eq!(
            wrapped("EX_NAME"),
            Some("new \\Pnlx\\Types\\String_(\"example\")")
        );
        assert_eq!(
            wrapped("EX_TITLE"),
            Some("new \\Pnlx\\Types\\String_(\"lib \" . \"example\" . \" v\")")
        );

        // The scalar variant unwraps a plain `int`/string, but keeps `float`/typed
        // values wrapped (an `f`-suffixed `float` is not a plain PHP scalar).
        assert_eq!(scalar("EX_FLAG_A"), Some("1"));
        assert_eq!(scalar("EX_NAME"), Some("\"example\""));
        assert_eq!(scalar("EX_RATIO"), Some("new \\Pnlx\\Types\\Float_(1.5)"));

        // Function-like macros are not emitted as constants themselves.
        assert_eq!(find("EX_MAX").map(|_| ()), None);
        assert_eq!(find("EX_POS_DISPLAY").map(|_| ()), None);

        // Enum constants are emitted too, wrapped as `int`s.
        assert_eq!(wrapped("EX_RED"), Some("new \\Pnlx\\Types\\Int_(0)"));
        assert_eq!(wrapped("EX_GREEN"), Some("new \\Pnlx\\Types\\Int_(1)"));
        assert_eq!(wrapped("EX_BLUE"), Some("new \\Pnlx\\Types\\Int_(2)"));
    }

    #[test]
    fn turns_function_like_macros_into_php_functions() {
        let functions = artifacts(HEADER).macro_functions;
        let find = |name: &str| functions.iter().find(|function| function.name == name);

        // A macro that calls this library's C function resolves to a static call.
        let double = find("EX_DOUBLE").expect("EX_DOUBLE function");
        assert_eq!(double.params, vec!["N".to_owned()]);
        assert_eq!(double.body, Ok("\\Pnlx\\Ex\\Ex::ex_add($N, $N)".to_owned()));

        // Nested calls and the literal/parameter mix are rendered positionally.
        let mix = find("EX_MIX").expect("EX_MIX function");
        assert_eq!(
            mix.body,
            Ok("\\Pnlx\\Ex\\Ex::ex_add(1, \\Pnlx\\Ex\\Ex::ex_add($X, $Y))".to_owned())
        );

        // A call to a C function this library does not define becomes a thrower.
        let bad = find("EX_BADCALL").expect("EX_BADCALL function");
        assert_eq!(bad.body, Err("ex_unknown".to_owned()));

        // EX_MAX uses `?:`, which has no allowed-operator rendering, so it is
        // dropped entirely rather than emitted.
        assert!(find("EX_MAX").is_none());

        // A prefix `&` (C address-of) has no faithful PHP rendering, so a macro
        // using it is dropped rather than emitting invalid `fn(& ( $N ))`.
        assert!(find("EX_TAKE_ADDR").is_none());

        // C concatenates adjacent operands in a macro body too; PHP needs `.`.
        let version = find("EX_VERSION_STR").expect("EX_VERSION_STR function");
        assert_eq!(version.body, Ok("$MJR . \".\" . $MNR".to_owned()));

        // A macro body with no operand (`()`) has no PHP expression, so it drops.
        assert!(find("EX_EMPTY_CONCAT").is_none());
    }

    #[test]
    fn rejects_multi_dot_version_number_constants() {
        // `#define X 1.2.3` is a C pp-number but not a valid PHP literal, so it
        // must be dropped rather than emitted as `const X = 1.2.3;`.
        let constants = artifacts(HEADER).constants;
        assert!(
            !constants
                .iter()
                .any(|constant| constant.name == "EX_BAD_VERSION"),
            "{constants:?}"
        );
    }

    #[test]
    fn resolves_dependency_functions_in_macros() {
        let mut dependency_functions = BTreeMap::new();
        dependency_functions.insert("ex_dep_call".to_owned(), "\\Pnlx\\Dep\\Dep".to_owned());

        let macro_functions = cdef_from_header(
            "#define EX_USE_DEP(X) ex_dep_call(X)\nint ex_local(int a);",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
                dependency_functions,
                ..Default::default()
            },
        )
        .expect("libclang must be available to run header_adapter tests")
        .macro_functions;

        // A C function this library lacks but a dependency provides resolves to a
        // static call on the dependency's class instead of a thrower.
        let use_dep = macro_functions
            .iter()
            .find(|function| function.name == "EX_USE_DEP")
            .expect("EX_USE_DEP function");
        assert_eq!(
            use_dep.body,
            Ok("\\Pnlx\\Dep\\Dep::ex_dep_call($X)".to_owned())
        );
    }

    #[test]
    fn derives_include_roots_for_nested_package_headers() {
        use std::path::PathBuf;
        // A package whose headers live below a system include root (libxml2)
        // contributes each intermediate directory as a `-I` root, so internal
        // `#include <libxml/...>` directives resolve to the real sub-headers.
        let dirs =
            super::include_search_dirs(&[PathBuf::from("/usr/include/libxml2/libxml/parser.h")]);
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/usr/include/libxml2/libxml"),
                PathBuf::from("/usr/include/libxml2"),
            ]
        );
    }

    #[test]
    fn fills_missing_pointer_base_with_void() {
        use super::fill_missing_pointer_base;
        // A bare `*` (base type dropped by libclang error recovery) becomes `void *`.
        assert_eq!(fill_missing_pointer_base("*"), "void *");
        assert_eq!(fill_missing_pointer_base("const *"), "void const *");
        // Types with a real base, or non-pointer types, are left untouched.
        assert_eq!(fill_missing_pointer_base("FILE *"), "FILE *");
        assert_eq!(fill_missing_pointer_base("const char *"), "const char *");
        assert_eq!(fill_missing_pointer_base("int"), "int");
    }

    #[test]
    fn array_parameters_decay_to_pointers() {
        use super::array_param_to_pointer;
        // C array parameters are pointers; rendering them so avoids "incomplete
        // struct" when the element type is opaque (libtiff `const TIFFFieldInfo arg1[]`).
        assert_eq!(
            array_param_to_pointer("const TIFFFieldInfo arg1[]"),
            "const TIFFFieldInfo *arg1"
        );
        assert_eq!(array_param_to_pointer("char arg3[1024]"), "char *arg3");
        assert_eq!(array_param_to_pointer("int *arr[]"), "int **arr");
        assert_eq!(array_param_to_pointer("int x"), "int x");
    }

    #[test]
    fn rewrites_unsupported_scalar_typedefs() {
        // `_Complex` and 128-bit integers are re-expressed as same-size structs.
        let cdef = cdef(
            "typedef _Complex double cplx;\n\
             typedef unsigned __int128 u128;\n\
             int ex_use(cplx a, u128 b);\n",
        );
        assert!(
            cdef.contains("typedef struct { double re; double im; } cplx;"),
            "{cdef}"
        );
        assert!(
            cdef.contains("typedef struct { uint64_t lo; uint64_t hi; } u128;"),
            "{cdef}"
        );
        assert!(
            !cdef.contains("_Complex") && !cdef.contains("__int128"),
            "{cdef}"
        );
    }

    #[test]
    fn skips_self_typedef_colliding_with_a_function() {
        // A struct tag and a function can share a name in C (separate namespaces)
        // but not in PHP FFI, so the convenience `typedef struct X X;` is dropped
        // (soxr's `soxr_quality_spec`, gnutls's `gnutls_random_art`).
        let cdef = cdef(
            "struct ex_spec { int p; };\n\
             int ex_spec(int recipe);\n",
        );
        assert!(!cdef.contains("typedef struct ex_spec ex_spec;"), "{cdef}");
        assert!(cdef.contains("int ex_spec(int recipe);"), "{cdef}");
    }

    #[test]
    fn aggregate_array_typedefs_drop_their_dimension() {
        // `typedef ex_plane ex_planes[3];` is the C by-reference array idiom; the
        // dimension is dropped so it parses regardless of element completeness.
        let cdef = cdef(
            "typedef struct ex_plane { int w; int h; } ex_plane;\n\
             typedef ex_plane ex_planes[3];\n\
             int ex_use(ex_planes p);\n",
        );
        assert!(cdef.contains("typedef ex_plane ex_planes;"), "{cdef}");
        assert!(!cdef.contains("ex_planes[3]"), "{cdef}");
    }

    #[test]
    fn adds_no_include_roots_for_headers_on_the_default_path() {
        use std::path::PathBuf;
        // Headers directly under a system include root are already searched by
        // libclang, so they need no extra `-I` roots.
        assert!(super::include_search_dirs(&[PathBuf::from("/usr/include/sodium.h")]).is_empty());
        assert!(
            super::include_search_dirs(&[PathBuf::from("/usr/local/include/zlib.h")]).is_empty()
        );
        // A header that does not live below any system include root (a fixture or
        // self-build checkout) contributes no roots, rather than walking up to `/`
        // and handing libclang bogus `-I` paths.
        assert!(
            super::include_search_dirs(&[PathBuf::from("/home/dev/proj/tests/example.h")])
                .is_empty()
        );
    }

    #[test]
    fn function_type_typedef_keeps_nested_function_pointer_parameters() {
        use super::typedef_declaration;
        // A function-*pointer* typedef: the name is woven into the first `(*)`.
        assert_eq!(
            typedef_declaration("cb", "void (*)(int, void *)"),
            "typedef void (*cb)(int, void *);"
        );
        // A function-*type* typedef: the name goes before the parameter list.
        assert_eq!(
            typedef_declaration("handler", "int (void *, size_t)"),
            "typedef int handler(void *, size_t);"
        );
        // A function-type typedef whose parameters are themselves function
        // pointers (openssl's `OSSL_FUNC_provider_register_child_cb_fn`): the
        // nested `(*)` belongs to a parameter and must NOT capture the typedef
        // name (the old `contains("(*)")` test mangled it).
        assert_eq!(
            typedef_declaration(
                "OSSL_FN",
                "int (const int *, int (*)(const void *, void *), void *)"
            ),
            "typedef int OSSL_FN(const int *, int (*)(const void *, void *), void *);"
        );
        // A plain pointer typedef is unaffected.
        assert_eq!(typedef_declaration("ptr", "void *"), "typedef void *ptr;");
    }

    #[test]
    fn renders_function_pointer_struct_fields_as_void_pointers() {
        // libmongoc's `_mongoc_stream_t` has many function-pointer fields; render
        // them as opaque `void *` so the struct loads (a function pointer is
        // pointer-sized and PHP can't invoke a struct-stored callback anyway).
        let cdef = cdef(
            "struct ex_stream {\n\
                 int type;\n\
                 void (*destroy)(struct ex_stream *s);\n\
                 int (*read)(struct ex_stream *s, void *buf);\n\
             };\n\
             typedef struct ex_stream ex_stream;\n\
             int ex_use(ex_stream *s);\n",
        );
        assert!(cdef.contains("struct ex_stream {"), "{cdef}");
        assert!(cdef.contains("void *destroy;"), "{cdef}");
        assert!(cdef.contains("void *read;"), "{cdef}");
        assert!(!cdef.contains("(*destroy)"), "{cdef}");
        assert!(!cdef.contains("(*read)"), "{cdef}");
    }

    #[test]
    fn array_parameter_of_typedef_pointer_decays_to_pointer() {
        // oniguruma's `onig_initialize(OnigEncoding encodings[], int)` where
        // `OnigEncoding` is `OnigEncodingType *`: the array parameter must decay to
        // `OnigEncoding *` (a pointer to the encoding pointer), not collapse to a
        // single `OnigEncoding` (which under-declares it by one pointer level).
        let cdef = cdef(
            "typedef struct ex_enc ex_enc;\n\
             typedef ex_enc *ex_encoding;\n\
             int ex_init(ex_encoding encodings[], int n);\n",
        );
        assert!(
            cdef.contains("int ex_init(ex_encoding *encodings, int n);"),
            "{cdef}"
        );
    }

    #[test]
    fn emits_exported_global_variables() {
        // Exported globals become `extern <type> <name>;` so a PHP example can take
        // their address (oniguruma's `OnigEncodingUTF8`) or read their value
        // (`OnigDefaultSyntax`, itself a pointer). `static` file-scope globals have no
        // exported symbol and are dropped.
        let cdef = cdef(
            "typedef struct ex_enc ex_enc;\n\
             extern ex_enc ex_encoding_utf8;\n\
             extern ex_enc *ex_default_syntax;\n\
             static int ex_internal_counter;\n\
             int ex_use(ex_enc *e);\n",
        );
        assert!(cdef.contains("extern ex_enc ex_encoding_utf8;"), "{cdef}");
        assert!(cdef.contains("extern ex_enc *ex_default_syntax;"), "{cdef}");
        assert!(!cdef.contains("ex_internal_counter"), "{cdef}");
    }

    #[test]
    fn renders_struct_with_callback_typedef_and_enum_fields() {
        // libconfig's `config_t` has a function-pointer-typedef field
        // (`config_include_fn_t include_fn`) and an enum-typedef field
        // (`config_error_t error_type`). Both must be rendered (callback → void*,
        // enum → its int typedef) so the struct is emitted, not left opaque.
        let cdef = cdef(
            "typedef const char **(*ex_include_fn)(int);\n\
             typedef enum ex_err { EX_OK = 0, EX_BAD = 1 } ex_err;\n\
             struct ex_cfg { int options; ex_include_fn include_fn; ex_err error_type; };\n\
             typedef struct ex_cfg ex_cfg;\n\
             void ex_init(ex_cfg *cfg);\n",
        );
        assert!(cdef.contains("struct ex_cfg {"), "{cdef}");
        assert!(cdef.contains("void *include_fn;"), "{cdef}");
        assert!(cdef.contains("ex_err error_type;"), "{cdef}");
        assert!(cdef.contains("typedef int ex_err;"), "{cdef}");
    }

    #[test]
    fn keeps_struct_opaque_when_it_embeds_an_incomplete_typedef_by_value() {
        // mbedtls embeds `mbedtls_x509_san_other_name` (a typedef for an
        // incomplete struct) by value inside an anonymous union, which slips past
        // the by-keyword check; PHP FFI can't size it, so the enclosing struct
        // must stay opaque.
        let cdef = cdef(
            "typedef struct ex_inner ex_inner;\n\
             struct ex_outer { int tag; union { ex_inner other; int code; } u; };\n\
             typedef struct ex_outer ex_outer;\n\
             int ex_use(ex_outer *o);\n",
        );
        assert!(cdef.contains("struct ex_outer;"), "{cdef}");
        assert!(!cdef.contains("struct ex_outer {"), "{cdef}");
        assert!(cdef.contains("int ex_use(ex_outer *o);"), "{cdef}");
    }

    #[test]
    fn rewrites_byte_pointer_parameters_to_char_pointers() {
        use super::rewrite_byte_pointer_params;
        let none = BTreeMap::new();
        // libidn2 `idn2_lookup_u8(const uint8_t *src, …)`: a single-level
        // `uint8_t *` parameter becomes `char *` so PHP FFI accepts a string; the
        // `uint8_t **` parameter and `int` are untouched.
        assert_eq!(
            rewrite_byte_pointer_params(
                "int idn2_lookup_u8(const uint8_t *src, uint8_t **lookupname, int flags);",
                &none
            ),
            "int idn2_lookup_u8(const char *src, uint8_t **lookupname, int flags);"
        );
        // `unsigned char *` and `signed char *` are also rejected by FFI for a
        // string, so both become `char *`.
        assert_eq!(
            rewrite_byte_pointer_params("int f(unsigned char *a, signed char *b);", &none),
            "int f(char *a, char *b);"
        );
        // `char *`, `void *`, and `int *` already work (or are not byte strings)
        // and are left alone.
        assert_eq!(
            rewrite_byte_pointer_params("int g(const char *s, void *p, int *n);", &none),
            "int g(const char *s, void *p, int *n);"
        );
        // Only parameters are rewritten — a byte-pointer *return* is left as is
        // (a string return is read with `FFI::string`, which works on any byte
        // pointer).
        assert_eq!(
            rewrite_byte_pointer_params("const uint8_t *h(uint8_t *in);", &none),
            "const uint8_t *h(char *in);"
        );
    }

    #[test]
    fn renders_function_returning_function_pointer_as_void_pointer() {
        // openssl `DSA_meth_get_sign` returns an *inline* function pointer; weaving
        // the name into the return declarator produced an invalid "function
        // returning function". Such a return collapses to an opaque `void *`. A
        // function-pointer *typedef* return (`ex_cb`) is pointer-sized and valid, so
        // it is left as is.
        let cdef = cdef(
            "typedef int (*ex_cb)(int);\n\
             ex_cb ex_get_cb(int slot);\n\
             int (*ex_get_handler(const char *name))(void *, int);\n",
        );
        assert!(
            cdef.contains("void *ex_get_handler(const char *name);"),
            "{cdef}"
        );
        assert!(!cdef.contains("(*ex_get_handler)"), "{cdef}");
        assert!(cdef.contains("ex_cb ex_get_cb(int slot);"), "{cdef}");
    }

    #[test]
    fn never_redefines_a_builtin_type_via_typedef() {
        use super::is_c_fundamental_type_name;
        // libmongoc's headers produce a spurious `typedef int bool(int *);` that
        // turns the builtin `bool` into a function type, so a `bool` struct field
        // becomes "function type is not allowed". A typedef over a fundamental type
        // name must never be emitted.
        assert!(is_c_fundamental_type_name("bool"));
        assert!(is_c_fundamental_type_name("int"));
        assert!(!is_c_fundamental_type_name("uint8_t"));
        assert!(!is_c_fundamental_type_name("OnigUChar"));
    }

    #[test]
    fn drops_object_like_macro_with_empty_parenthesised_body() {
        // mongoc's `#define MONGOC_PRERELEASE_VERSION ()` renders as `( )`, which is
        // not a PHP expression; the constant must be dropped, not emitted.
        let constants = artifacts(
            "#define EX_PRERELEASE ()\n\
             #define EX_REAL 7\n\
             int ex_use(int x);\n",
        )
        .constants;
        assert!(
            !constants
                .iter()
                .any(|constant| constant.name == "EX_PRERELEASE"),
            "{constants:?}"
        );
        assert!(
            constants.iter().any(|constant| constant.name == "EX_REAL"
                && constant.wrapped == "new \\Pnlx\\Types\\Int_(7)"),
            "{constants:?}"
        );
    }

    #[test]
    fn rewrites_byte_pointer_parameters_hidden_behind_a_typedef() {
        use super::rewrite_byte_pointer_params;
        let mut typedefs = BTreeMap::new();
        // oniguruma: `OnigUChar` is `unsigned char`, used as `const OnigUChar *`.
        typedefs.insert("OnigUChar".to_owned(), "unsigned char".to_owned());
        // pcre2: `PCRE2_SPTR8` is itself a byte pointer, used directly.
        typedefs.insert("PCRE2_SPTR8".to_owned(), "const unsigned char *".to_owned());
        // A `char`-based typedef must NOT be rewritten — `char *` already works.
        typedefs.insert("tmbchar".to_owned(), "char".to_owned());

        assert_eq!(
            rewrite_byte_pointer_params("int onig_search(const OnigUChar *pattern);", &typedefs),
            "int onig_search(const char *pattern);"
        );
        assert_eq!(
            rewrite_byte_pointer_params("int pcre2_compile(PCRE2_SPTR8 pattern);", &typedefs),
            "int pcre2_compile(const char *pattern);"
        );
        // `char`-based typedef pointers are left untouched.
        assert_eq!(
            rewrite_byte_pointer_params("int tidy(const tmbchar *s);", &typedefs),
            "int tidy(const tmbchar *s);"
        );
    }
}
