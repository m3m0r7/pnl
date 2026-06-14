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

#[derive(Debug, Clone)]
pub struct HeaderAdapterOptions {
    pub symbol_prefix: String,
}

/// The two things extracted from a C header: the cdef text for PHP `FFI::cdef`,
/// and the object-like `#define` constants worth surfacing as PHP `const`s.
#[derive(Debug, Default)]
pub struct HeaderArtifacts {
    pub cdef: String,
    /// `(name, php_value_expression)` pairs, in source order, for `const.php`.
    pub constants: Vec<(String, String)>,
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
            constants: Vec::new(),
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
    Ok(HeaderArtifacts {
        cdef: render(&collected, prefix),
        constants: macro_constants(&collected.macros, prefix),
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
    tokens: Vec<(TokenKind, String)>,
}

#[derive(Default)]
struct Collected {
    structs: BTreeMap<String, String>,
    enums: BTreeSet<String>,
    typedefs: Vec<String>,
    functions: Vec<String>,
    macros: Vec<RawMacro>,
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
            EntityKind::EnumDecl => {
                if let Some(name) = entity.get_name() {
                    collected.enums.insert(name);
                }
            }
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

/// Record an object-like `#define`. Function-like and builtin macros, and any
/// with no replacement (e.g. the empty `#define`s used to neutralise ABI
/// macros), are skipped. The macro's own name is the first token, so the
/// replacement is everything after it.
fn collect_macro(entity: &Entity<'_>, collected: &mut Collected) {
    if entity.is_function_like_macro() || entity.is_builtin_macro() {
        return;
    }
    let Some(name) = entity.get_name() else {
        return;
    };
    let Some(range) = entity.get_range() else {
        return;
    };
    let tokens: Vec<(TokenKind, String)> = range
        .tokenize()
        .iter()
        .map(|token| (token.get_kind(), token.get_spelling()))
        .skip_while(|(_, spelling)| spelling == &name)
        .collect();
    if tokens.is_empty() {
        return;
    }
    collected.macros.push(RawMacro { name, tokens });
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

/// Translate the prefix-matching object-like macros into `(name, php_value)`
/// pairs, keeping only those whose replacement is a constant expression PHP can
/// evaluate. A single forward pass over the source-ordered macros lets a later
/// macro reference an already-emitted one by name (e.g. composite flag masks),
/// while anything untranslatable (casts, char literals, calls, unknown names) is
/// silently dropped.
fn macro_constants(macros: &[RawMacro], prefix: &str) -> Vec<(String, String)> {
    let needle = prefix.to_ascii_lowercase();
    let mut emitted = BTreeSet::new();
    let mut constants = Vec::new();
    for macro_def in macros {
        if !macro_def.name.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        if let Some(value) = translate_macro_value(&macro_def.tokens, &emitted) {
            emitted.insert(macro_def.name.clone());
            constants.push((macro_def.name.clone(), value));
        }
    }
    constants
}

/// Translate macro replacement tokens into a PHP constant expression, or `None`
/// if any token is not a literal, an allowed arithmetic/bitwise operator, or a
/// reference to an already-emitted constant.
fn translate_macro_value(
    tokens: &[(TokenKind, String)],
    emitted: &BTreeSet<String>,
) -> Option<String> {
    let mut parts = Vec::with_capacity(tokens.len());
    for (kind, spelling) in tokens {
        let part = match kind {
            TokenKind::Literal => translate_literal(spelling)?,
            TokenKind::Punctuation if is_allowed_operator(spelling) => spelling.clone(),
            TokenKind::Identifier if emitted.contains(spelling) => spelling.clone(),
            _ => return None,
        };
        parts.push(part);
    }
    Some(parts.join(" "))
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
        let digits = hex.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| format!("0x{digits}"));
    }
    if let Some(binary) = lower.strip_prefix("0b") {
        let digits = binary.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b == b'0' || b == b'1'))
            .then(|| format!("0b{digits}"));
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
            },
        )
        .unwrap();
        assert_eq!(out.cdef, "int whatever(void);");
        assert!(out.constants.is_empty());
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

        // Function-like macros cannot be translated to a PHP const.
        assert_eq!(lookup("EX_MAX"), None);
        // Enum constants are emitted via the enum projection, not as macros.
        assert_eq!(lookup("EX_RED"), None);
    }
}
