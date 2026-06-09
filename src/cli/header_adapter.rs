use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use clang::{Clang, Entity, EntityKind, Index, Linkage, TypeKind};

/// libclang holds process-global state and the `clang` crate only permits a
/// single live [`Clang`] token at a time, so every translation is serialised.
static CLANG_GUARD: Mutex<()> = Mutex::new(());

/// Self-contained type aliases prepended to the generated cdef. PHP's
/// `FFI::cdef` has no access to system headers, so these stand in for the
/// fixed-width and SDL-style integer types the real headers rely on. The same
/// text is fed to libclang while parsing so those names resolve there too.
const PRELUDE: &str = concat!(
    "typedef signed long ssize_t;\n",
    "typedef unsigned long size_t;\n",
    "typedef long intptr_t;\n",
    "typedef unsigned char uint8_t;\n",
    "typedef unsigned short uint16_t;\n",
    "typedef unsigned int uint32_t;\n",
    "typedef unsigned long long uint64_t;\n",
    "typedef unsigned char Uint8;\n",
    "typedef unsigned short Uint16;\n",
    "typedef unsigned int Uint32;\n",
);

#[derive(Debug, Clone)]
pub struct HeaderAdapterOptions {
    pub symbol_prefix: String,
}

/// Translate a C header into a normalised cdef suitable for PHP `FFI::cdef`,
/// keeping only the declarations whose names start with `symbol_prefix`.
///
/// Parsing is delegated to libclang; only declaration discovery and the
/// FFI-specific re-emission are performed here.
pub fn cdef_from_header(header: &str, options: &HeaderAdapterOptions) -> Result<String> {
    let prefix = options.symbol_prefix.trim();
    if prefix.is_empty() {
        return Ok(header.to_owned());
    }

    let _guard = CLANG_GUARD
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let clang = Clang::new().map_err(|err| anyhow!("failed to initialise libclang: {err}"))?;
    let index = Index::new(&clang, false, false);
    let workspace = Workspace::new()?;
    let prelude_path = workspace.write("prelude.h", PRELUDE)?;

    let collected = parse_with_neutralized_macros(&index, &workspace, &prelude_path, header)?;
    Ok(render(&collected, prefix))
}

/// Parse the header, neutralising the ABI/attribute macros (e.g. `DECLSPEC`,
/// `SDLCALL`, `LIBUSB_CALL`, `LIBUSB_PACKED`) that appear in unpreprocessed
/// headers but whose definitions live in headers we are not given. They are
/// replaced with empty `#define`s so libclang can parse the declarations.
///
/// Candidates are found two ways: tokens that look like ABI macros (all-caps
/// call-convention / attribute names), and any remaining `unknown type name`
/// libclang reports. Enum constants are never neutralised, and a macro the
/// header defines itself wins over our empty definition (later `#define` wins),
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
    let mut candidates = BTreeSet::new();
    for token in identifier_tokens(header) {
        if is_abi_macro_token(token) {
            candidates.insert(token.to_owned());
        }
    }
    candidates
}

fn is_abi_macro_token(token: &str) -> bool {
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

#[derive(Default)]
struct Collected {
    structs: BTreeMap<String, String>,
    enums: BTreeSet<String>,
    typedefs: Vec<String>,
    functions: Vec<String>,
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
            _ => {}
        }
    }

    collected.functions.sort();
    collected.functions.dedup();
    collected.typedefs.sort();
    collected.typedefs.dedup();
    collected
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
    use super::{HeaderAdapterOptions, cdef_from_header};

    fn cdef(header: &str) -> String {
        cdef_from_header(
            header,
            &HeaderAdapterOptions {
                symbol_prefix: "ex_".to_owned(),
            },
        )
        .expect("libclang must be available to run header_adapter tests")
    }

    const HEADER: &str = r#"
        /* example device library */
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
        .unwrap();

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
        assert_eq!(out, "int whatever(void);");
    }
}
