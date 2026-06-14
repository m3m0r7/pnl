use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use clang::token::TokenKind;
use clang::{Clang, Entity, EntityKind, Index, Linkage, TypeKind};

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
const PRELUDE: &str = include_str!("templates/header/prelude.h");

/// A lexer token kept for macro analysis: its kind and its spelling.
type Token = (TokenKind, String);

#[derive(Debug, Clone)]
pub struct HeaderAdapterOptions {
    pub symbol_prefix: String,
    /// The generated entity's fully-qualified class name (e.g. `\Pnlx\Libsdl\Libsdl`),
    /// used to render the C-function calls inside function-like macros.
    pub entity_fqcn: String,
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
    /// `(name, php_value_expression)` pairs, in source order, for `const.php`.
    pub constants: Vec<(String, String)>,
    pub macro_functions: Vec<MacroFunction>,
}

/// Translate a C header into a normalised cdef suitable for PHP `FFI::cdef`,
/// keeping only the declarations whose names contain `symbol_prefix`, and
/// extract its object-like `#define` constants.
///
/// Parsing is delegated to libclang; only declaration discovery and the
/// FFI-specific re-emission are performed here.
pub fn cdef_from_header(header: &str, options: &HeaderAdapterOptions) -> Result<HeaderArtifacts> {
    let prefix = options.symbol_prefix.trim();
    if prefix.is_empty() {
        return Ok(HeaderArtifacts {
            cdef: header.to_owned(),
            ..HeaderArtifacts::default()
        });
    }

    let _guard = CLANG_GUARD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let clang = Clang::new().map_err(|err| anyhow!("failed to initialise libclang: {err}"))?;
    let index = Index::new(&clang, false, false);
    let workspace = Workspace::new()?;
    let prelude_path = workspace.write("prelude.h", PRELUDE)?;

    let collected = parse_with_neutralized_macros(&index, &workspace, &prelude_path, header)?;
    let constants = header_constants(&collected, prefix);
    let constant_names: BTreeSet<String> = constants.iter().map(|(name, _)| name.clone()).collect();
    Ok(HeaderArtifacts {
        cdef: render(&collected, prefix),
        macro_functions: macro_functions(&collected, prefix, &constant_names, &options.entity_fqcn),
        constants,
    })
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
fn parse_with_neutralized_macros(
    index: &Index<'_>,
    workspace: &Workspace,
    prelude_path: &Path,
    header: &str,
) -> Result<Collected> {
    let arguments = [
        "-x",
        "c",
        "-std=c11",
        "-ferror-limit=0",
        // Keep `#define`s in the AST as MacroDefinition cursors so object-like
        // constant macros can be re-emitted as PHP `const`s.
        "-Xclang",
        "-detailed-preprocessing-record",
        "-include",
        prelude_path.to_str().context("prelude path is not UTF-8")?,
    ];
    let parse = |source: &str| -> Result<_> {
        let header_path = workspace.write("header.h", source)?;
        index
            .parser(&header_path)
            .arguments(&arguments)
            .skip_function_bodies(true)
            .parse()
            .context("libclang failed to parse the header")
    };

    // A lenient first pass: enums parse even while ABI macros are undefined,
    // giving us the enum constants that must be excluded from neutralisation.
    let enum_constants = gather_enum_constants(&parse(header)?.get_entity());
    let mut defines: BTreeSet<String> = abi_macro_candidates(header)
        .difference(&enum_constants)
        .cloned()
        .collect();

    for _ in 0..32 {
        let unit = parse(&neutralized_source(header, &defines))?;

        let mut discovered = false;
        for diagnostic in unit.get_diagnostics() {
            if let Some(name) = unknown_type_name(&diagnostic.get_text())
                && !enum_constants.contains(&name)
                && defines.insert(name)
            {
                discovered = true;
            }
        }

        if !discovered {
            return Ok(collect(&unit.get_entity()));
        }
    }

    Err(anyhow!(
        "header still failed to parse after neutralising macros"
    ))
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
    let mut candidates = BTreeSet::new();
    for token in identifier_tokens(header) {
        if is_abi_macro_token(token) && !defined.contains(token) {
            candidates.insert(token.to_owned());
        }
    }
    candidates
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

fn neutralized_source(header: &str, defines: &BTreeSet<String>) -> String {
    let mut source = String::new();
    for name in defines {
        source.push_str("#define ");
        source.push_str(name);
        source.push('\n');
    }
    source.push_str(header);
    source
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
    enums: BTreeSet<String>,
    typedefs: Vec<String>,
    functions: Vec<String>,
    macros: Vec<RawMacro>,
    /// Function-like macros by name, for constant-argument expansion.
    fn_macros: BTreeMap<String, RawFnMacro>,
    /// Names of the kept (externally-linked) C functions, for resolving the
    /// calls inside function-like macros.
    function_names: BTreeSet<String>,
    /// `(name, value)` for every enumerator, in source order, for `const.php`.
    enum_constants: Vec<(String, i64)>,
}

fn collect(translation_unit: &Entity<'_>) -> Collected {
    let mut collected = Collected::default();

    for entity in translation_unit.get_children() {
        if !entity.is_in_main_file() {
            continue;
        }

        match entity.get_kind() {
            EntityKind::FunctionDecl => collect_function(&entity, &mut collected),
            EntityKind::StructDecl => collect_struct(&entity, &mut collected),
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

/// Record the enum's name (projected onto `int` in the cdef) and each of its
/// enumerators with its evaluated integer value (for `const.php`).
fn collect_enum(entity: &Entity<'_>, collected: &mut Collected) {
    if let Some(name) = entity.get_name() {
        collected.enums.insert(name);
    }
    for constant in entity.get_children() {
        if constant.get_kind() != EntityKind::EnumConstantDecl {
            continue;
        }
        if let (Some(name), Some((signed, _))) =
            (constant.get_name(), constant.get_enum_constant_value())
        {
            collected.enum_constants.push((name, signed));
        }
    }
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

fn collect_function(entity: &Entity<'_>, collected: &mut Collected) {
    if entity.is_variadic() {
        return;
    }
    // `static inline` helpers have no exported symbol, so binding them through
    // FFI would fail; only keep externally-linked functions.
    if entity.get_linkage() != Some(Linkage::External) {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    let return_type = entity
        .get_result_type()
        .map(|ty| ty.get_display_name())
        .unwrap_or_else(|| "void".to_owned());

    let params = entity
        .get_arguments()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let type_name = argument
                .get_type()
                .map(|ty| ty.get_display_name())
                .unwrap_or_default();
            let param_name = argument.get_name().unwrap_or_else(|| format!("arg{index}"));
            declarator(&type_name, &param_name)
        })
        .collect::<Vec<_>>();

    let params = if params.is_empty() {
        "void".to_owned()
    } else {
        params.join(", ")
    };

    collected.function_names.insert(name.clone());
    collected
        .functions
        .push(format!("{}({});", declarator(&return_type, &name), params));
}

fn collect_struct(entity: &Entity<'_>, collected: &mut Collected) {
    if !entity.is_definition() {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };

    // If any field cannot be rendered cleanly, leave the struct opaque
    // (forward-declared) instead of emitting an invalid definition.
    let fields: Option<Vec<String>> = entity
        .get_children()
        .into_iter()
        .filter(|child| child.get_kind() == EntityKind::FieldDecl)
        .map(|field| render_struct_field(&field))
        .collect();

    if let Some(fields) = fields {
        collected.structs.insert(
            name.clone(),
            format!("struct {name} {{ {} }};", fields.join(" ")),
        );
    }
}

/// Render one struct field, recursing into anonymous nested unions/structs and
/// weaving the field name into function-pointer types. Returns `None` for a
/// field libclang can only describe by source location (which would produce
/// invalid C), signalling that the whole struct should stay opaque.
fn render_struct_field(field: &Entity<'_>) -> Option<String> {
    let name = field.get_name()?;
    let field_type = field.get_type()?;

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
    if display.contains("(*)") {
        // Function-pointer field: `ret (*name)(args)`.
        return Some(format!(
            "{};",
            display.replacen("(*)", &format!("(*{name})"), 1)
        ));
    }
    Some(format!("{};", declarator(&display, &name)))
}

fn collect_typedef(entity: &Entity<'_>, collected: &mut Collected) {
    let Some(name) = entity.get_name() else {
        return;
    };
    let Some(underlying) = entity.get_typedef_underlying_type() else {
        return;
    };

    match underlying.get_canonical_type().get_kind() {
        // Enums are projected onto `int`, which is what FFI cares about.
        TypeKind::Enum => {
            collected.enums.insert(name);
        }
        // Struct aliases are emitted from the struct section to avoid duplicates.
        TypeKind::Record => {}
        _ => collected
            .typedefs
            .push(typedef_declaration(&name, &underlying.get_display_name())),
    }
}

/// Combine a type and an identifier into a C declarator, attaching pointers and
/// array extents to the identifier the way C syntax requires.
fn declarator(type_name: &str, identifier: &str) -> String {
    let type_name = type_name.trim();
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
    if underlying.contains("(*)") {
        format!(
            "typedef {};",
            underlying.replacen("(*)", &format!("(*{name})"), 1)
        )
    } else {
        format!("typedef {};", declarator(underlying, name))
    }
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
fn header_constants(collected: &Collected, prefix: &str) -> Vec<(String, String)> {
    let needle = prefix.to_ascii_lowercase();
    let mut emitted = BTreeSet::new();
    let mut constants = Vec::new();

    for (name, value) in &collected.enum_constants {
        if name.to_ascii_lowercase().contains(&needle) && emitted.insert(name.clone()) {
            constants.push((name.clone(), value.to_string()));
        }
    }

    for macro_def in &collected.macros {
        if !macro_def.name.to_ascii_lowercase().contains(&needle)
            || emitted.contains(&macro_def.name)
        {
            continue;
        }
        if let Some(value) =
            translate_macro_value(&macro_def.tokens, &emitted, &collected.fn_macros)
        {
            emitted.insert(macro_def.name.clone());
            constants.push((macro_def.name.clone(), value));
        }
    }

    constants
}

/// Bounds nested function-like-macro expansions.
const MAX_MACRO_EXPANSION_DEPTH: usize = 16;

/// Translate macro replacement tokens into a PHP constant expression, expanding
/// any constant-argument calls to known function-like macros. Returns `None` on
/// anything untranslatable.
fn translate_macro_value(
    tokens: &[Token],
    emitted: &BTreeSet<String>,
    fn_macros: &BTreeMap<String, RawFnMacro>,
) -> Option<String> {
    expand_tokens(tokens, emitted, fn_macros, 0)
}

/// Render a token stream as a PHP constant expression. Literals/operators pass
/// through; an identifier resolves to an already-emitted constant; an
/// identifier *called* with constant arguments (`DISPLAY(0)`) is expanded by
/// substituting into the known function-like macro's body. Anything else (char
/// literals, unknown names, calls to non-macro functions, excessive nesting)
/// yields `None`.
fn expand_tokens(
    tokens: &[Token],
    emitted: &BTreeSet<String>,
    fn_macros: &BTreeMap<String, RawFnMacro>,
    depth: usize,
) -> Option<String> {
    if depth > MAX_MACRO_EXPANSION_DEPTH {
        return None;
    }
    let mut parts = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (kind, spelling) = &tokens[index];
        match kind {
            TokenKind::Literal => {
                parts.push(translate_literal(spelling)?);
                index += 1;
            }
            TokenKind::Punctuation if is_allowed_operator(spelling) => {
                parts.push(spelling.clone());
                index += 1;
            }
            TokenKind::Identifier if tokens.get(index + 1).is_some_and(|(_, next)| next == "(") => {
                let macro_def = fn_macros.get(spelling)?;
                let (args, after) = parse_call_args(tokens, index + 1)?;
                if args.len() != macro_def.params.len() {
                    return None;
                }
                let substituted = substitute_params(&macro_def.body, &macro_def.params, &args);
                let expanded = expand_tokens(&substituted, emitted, fn_macros, depth + 1)?;
                parts.push(format!("({expanded})"));
                index = after;
            }
            TokenKind::Identifier if emitted.contains(spelling) => {
                parts.push(spelling.clone());
                index += 1;
            }
            _ => return None,
        }
    }
    Some(parts.join(" "))
}

/// Parse a parenthesised, comma-separated argument list starting at the `(` at
/// `open`. Returns each argument's tokens and the index just past the matching
/// `)`, or `None` if the parentheses are unbalanced.
fn parse_call_args(tokens: &[Token], open: usize) -> Option<(Vec<Vec<Token>>, usize)> {
    let mut depth = 0usize;
    let mut args: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    for (offset, token) in tokens[open..].iter().enumerate() {
        match token.1.as_str() {
            "(" => {
                depth += 1;
                if depth > 1 {
                    current.push(token.clone());
                }
            }
            ")" => {
                depth -= 1;
                if depth == 0 {
                    if !current.is_empty() || !args.is_empty() {
                        args.push(current);
                    }
                    return Some((args, open + offset + 1));
                }
                current.push(token.clone());
            }
            "," if depth == 1 => args.push(std::mem::take(&mut current)),
            _ => current.push(token.clone()),
        }
    }
    None
}

/// Replace each parameter identifier in a macro body with its argument's tokens.
fn substitute_params(body: &[Token], params: &[String], args: &[Vec<Token>]) -> Vec<Token> {
    let mut out = Vec::new();
    for token in body {
        if token.0 == TokenKind::Identifier
            && let Some(position) = params.iter().position(|param| *param == token.1)
        {
            out.extend(args[position].iter().cloned());
        } else {
            out.push(token.clone());
        }
    }
    out
}

/// PHP-compatible rendering of a single C literal token (`0x20`, `1u`, `1.5f`,
/// `"text"`). Char literals and anything unrecognised yield `None`.
fn translate_literal(literal: &str) -> Option<String> {
    match literal.chars().next()? {
        '"' => Some(literal.to_owned()),
        '\'' => None,
        _ => translate_number(literal),
    }
}

/// Strip C integer/float suffixes and validate the remaining numeric literal so
/// PHP reads the same value. Hex/binary keep their `0x`/`0b` prefix; the `f`/`F`
/// hex digits are never mistaken for a float suffix.
fn translate_number(literal: &str) -> Option<String> {
    let lower = literal.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        // Validate on the lowercased form but keep the original digit case.
        let digits = hex.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| format!("0x{}", &literal[2..2 + digits.len()]));
    }
    if let Some(binary) = lower.strip_prefix("0b") {
        let digits = binary.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b == b'0' || b == b'1'))
            .then(|| format!("0b{}", &literal[2..2 + digits.len()]));
    }
    let trimmed = lower.trim_end_matches(['u', 'l', 'f']);
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'+' | b'-'))
    {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Operators safe to pass through verbatim into a PHP constant expression (they
/// have the same meaning and precedence as in C).
fn is_allowed_operator(token: &str) -> bool {
    matches!(
        token,
        "(" | ")" | "+" | "-" | "*" | "/" | "%" | "<<" | ">>" | "&" | "|" | "^" | "~"
    )
}

/// Why a function-like macro body could not be rendered as a PHP expression.
enum FnBodyError {
    /// It calls a C function that is not in this library (named here).
    UndefinedCall(String),
    /// It uses something with no PHP equivalent (cast, char literal, unknown name).
    Untranslatable,
}

/// Turn each prefix-matching function-like macro into a [`MacroFunction`].
///
/// A macro whose body renders cleanly becomes a `return <expr>;` function; one
/// that calls a C function this library does not define becomes a throwing
/// function; anything untranslatable is dropped.
fn macro_functions(
    collected: &Collected,
    prefix: &str,
    consts: &BTreeSet<String>,
    entity_fqcn: &str,
) -> Vec<MacroFunction> {
    let needle = prefix.to_ascii_lowercase();
    // Constants live in the entity's namespace (the parent of the entity class).
    let const_namespace = entity_fqcn
        .rsplit_once('\\')
        .map_or(entity_fqcn, |(namespace, _)| namespace);

    let mut functions = Vec::new();
    for (name, macro_def) in &collected.fn_macros {
        if !name.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        let params: BTreeSet<&str> = macro_def.params.iter().map(String::as_str).collect();
        let body = match render_fn_body(
            &macro_def.body,
            &params,
            consts,
            &collected.function_names,
            &collected.fn_macros,
            entity_fqcn,
            const_namespace,
            0,
        ) {
            Ok(expr) => Ok(expr),
            Err(FnBodyError::UndefinedCall(symbol)) => Err(symbol),
            Err(FnBodyError::Untranslatable) => continue,
        };
        functions.push(MacroFunction {
            name: name.clone(),
            params: macro_def.params.clone(),
            body,
        });
    }
    functions
}

/// Render a function-like macro body as a PHP expression: parameters become
/// `$param`, this library's C functions become `<Class>::fn(...)` static calls,
/// known constants become fully-qualified references, and a nested function-like
/// macro call is expanded inline.
#[allow(clippy::too_many_arguments)]
fn render_fn_body(
    tokens: &[Token],
    params: &BTreeSet<&str>,
    consts: &BTreeSet<String>,
    functions: &BTreeSet<String>,
    fn_macros: &BTreeMap<String, RawFnMacro>,
    entity_fqcn: &str,
    const_namespace: &str,
    depth: usize,
) -> std::result::Result<String, FnBodyError> {
    if depth > MAX_MACRO_EXPANSION_DEPTH {
        return Err(FnBodyError::Untranslatable);
    }
    let mut parts = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (kind, spelling) = &tokens[index];
        let is_call = tokens.get(index + 1).is_some_and(|(_, next)| next == "(");
        match kind {
            TokenKind::Literal => {
                parts.push(translate_literal(spelling).ok_or(FnBodyError::Untranslatable)?);
                index += 1;
            }
            TokenKind::Punctuation if is_allowed_operator(spelling) => {
                parts.push(spelling.clone());
                index += 1;
            }
            TokenKind::Identifier if is_call => {
                let (args, after) =
                    parse_call_args(tokens, index + 1).ok_or(FnBodyError::Untranslatable)?;
                let recurse = |body: &[Token], depth: usize| {
                    render_fn_body(
                        body,
                        params,
                        consts,
                        functions,
                        fn_macros,
                        entity_fqcn,
                        const_namespace,
                        depth,
                    )
                };
                if let Some(macro_def) = fn_macros.get(spelling) {
                    if args.len() != macro_def.params.len() {
                        return Err(FnBodyError::Untranslatable);
                    }
                    let substituted = substitute_params(&macro_def.body, &macro_def.params, &args);
                    parts.push(format!("({})", recurse(&substituted, depth + 1)?));
                } else if functions.contains(spelling) {
                    let rendered = args
                        .iter()
                        .map(|arg| recurse(arg, depth + 1))
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    parts.push(format!(
                        "{entity_fqcn}::{spelling}({})",
                        rendered.join(", ")
                    ));
                } else {
                    return Err(FnBodyError::UndefinedCall(spelling.clone()));
                }
                index = after;
            }
            TokenKind::Identifier if params.contains(spelling.as_str()) => {
                parts.push(format!("${spelling}"));
                index += 1;
            }
            TokenKind::Identifier if consts.contains(spelling) => {
                parts.push(format!("{const_namespace}\\{spelling}"));
                index += 1;
            }
            _ => return Err(FnBodyError::Untranslatable),
        }
    }
    Ok(parts.join(" "))
}

fn render(collected: &Collected, prefix: &str) -> String {
    let typedefs = collected
        .typedefs
        .iter()
        .map(|typedef| replace_prefixed_enums(typedef, &collected.enums))
        .collect::<Vec<_>>();
    let functions = collected
        .functions
        .iter()
        .filter(|function| function.contains(prefix))
        .map(|function| replace_prefixed_enums(function, &collected.enums))
        .collect::<Vec<_>>();
    let referenced = referenced_structs(&typedefs, &functions, &collected.structs);

    let mut out = String::new();
    out.push_str(PRELUDE);
    out.push('\n');

    out.push_str("struct timeval;\n");
    for name in referenced.iter().chain(collected.structs.keys()) {
        out.push_str("struct ");
        out.push_str(name);
        out.push_str(";\n");
    }

    out.push('\n');
    for name in collected.structs.keys() {
        out.push_str(&format!("typedef struct {name} {name};\n"));
    }
    for name in &collected.enums {
        out.push_str(&format!("typedef int {name};\n"));
    }
    for typedef in &typedefs {
        out.push_str(typedef);
        out.push('\n');
    }

    out.push('\n');
    out.push_str("struct timeval { long tv_sec; int tv_usec; };\n");
    for definition in collected.structs.values() {
        out.push_str(&replace_prefixed_enums(definition, &collected.enums));
        out.push('\n');
    }

    out.push('\n');
    for function in &functions {
        out.push_str(function);
        out.push('\n');
    }

    out
}

/// Collect struct names that are referenced by the kept declarations but are
/// not defined here, so they can be forward-declared.
fn referenced_structs(
    typedefs: &[String],
    functions: &[String],
    defined: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut referenced = BTreeSet::new();
    for declaration in typedefs.iter().chain(functions.iter()) {
        let mut rest = declaration.as_str();
        while let Some(index) = rest.find("struct ") {
            rest = &rest[index + "struct ".len()..];
            let name = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect::<String>();
            if !name.is_empty() && name != "timeval" && !defined.contains_key(&name) {
                referenced.insert(name);
            }
        }
    }
    referenced
}

/// Replace `enum <name>` references (for enums projected onto `int`) with `int`.
fn replace_prefixed_enums(input: &str, enums: &BTreeSet<String>) -> String {
    let mut out = input.to_owned();
    // Longest names first so a name that is a prefix of another is not mangled.
    for name in enums.iter().rev() {
        out = out.replace(&format!("enum {name}"), "int");
    }
    out
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
    use super::{HeaderAdapterOptions, HeaderArtifacts, cdef_from_header};

    fn artifacts(header: &str) -> HeaderArtifacts {
        cdef_from_header(
            header,
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
            },
        )
        .expect("libclang must be available to run header_adapter tests")
    }

    fn cdef(header: &str) -> String {
        artifacts(header).cdef
    }

    const HEADER: &str = r#"
        /* example device library */
        #define EX_FLAG_A 0x01
        #define EX_FLAG_B (1 << 1)
        #define EX_FLAGS (EX_FLAG_A | EX_FLAG_B)
        #define EX_NAME "example"
        #define EX_RATIO 1.5f
        #define EX_MAX(a, b) ((a) > (b) ? (a) : (b))
        #define EX_POS_MASK 0x2FFF0000
        #define EX_POS_DISPLAY(X) (EX_POS_MASK | (X))
        #define EX_POS_CENTERED EX_POS_DISPLAY(0)
        #define EX_DOUBLE(N) ex_add(N, N)
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
    fn drops_inline_and_variadic_functions() {
        let cdef = cdef(HEADER);
        assert!(!cdef.contains("ex_helper"), "inline kept: {cdef}");
        assert!(!cdef.contains("ex_printf"), "variadic kept: {cdef}");
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
    fn emits_anonymous_union_struct_fields_inline() {
        // Mirrors SDL's SDL_GameControllerButtonBind, whose anonymous union once
        // produced an invalid `union (unnamed union at ...)` field.
        let cdef = cdef_from_header(
            "struct ex_bind { int kind; union { int button; int axis; } value; };",
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
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
        let signatures = crate::generate::parse_function_signatures(&cdef);
        assert!(
            signatures.iter().all(|sig| sig.name.starts_with("ex_")),
            "unexpected signatures: {:?}",
            signatures.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn returns_header_unchanged_without_prefix() {
        let out = cdef_from_header(
            "int whatever(void);",
            &HeaderAdapterOptions {
                symbol_prefix: "  ".to_owned(),
                entity_fqcn: "\\Pnlx\\Ex\\Ex".to_owned(),
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
        let lookup = |name: &str| {
            constants
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };

        // Integer, hex, shift, and a composite that references earlier constants.
        assert_eq!(lookup("EX_FLAG_A"), Some("0x01"));
        assert_eq!(lookup("EX_FLAG_B"), Some("( 1 << 1 )"));
        assert_eq!(lookup("EX_FLAGS"), Some("( EX_FLAG_A | EX_FLAG_B )"));
        assert_eq!(lookup("EX_NAME"), Some("\"example\""));
        assert_eq!(lookup("EX_RATIO"), Some("1.5"));

        // Function-like macros are not emitted as constants themselves …
        assert_eq!(lookup("EX_MAX"), None);
        assert_eq!(lookup("EX_POS_DISPLAY"), None);
        // … but an object-like macro that calls one with constant arguments is
        // expanded by substituting the body (EX_POS_DISPLAY(0) → (MASK | (0))).
        assert_eq!(lookup("EX_POS_MASK"), Some("0x2FFF0000"));
        assert_eq!(lookup("EX_POS_CENTERED"), Some("(( EX_POS_MASK | ( 0 ) ))"));

        // Enum constants are emitted too, with their evaluated integer values.
        assert_eq!(lookup("EX_RED"), Some("0"));
        assert_eq!(lookup("EX_GREEN"), Some("1"));
        assert_eq!(lookup("EX_BLUE"), Some("2"));
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
    }
}
