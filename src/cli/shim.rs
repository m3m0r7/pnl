//! Compiled trampoline shims for `static inline` C functions.
//!
//! A `static inline` function is defined in a header and exports no symbol, so FFI
//! cannot bind it — pnl normally surfaces it as a throwing stub. When the consumer
//! opts in via `compile_options.static_inline`, this module emits a tiny C file that
//! `#include`s the package headers and defines, for each such function, a real
//! exported trampoline (`pnl_si_<name>`) that just forwards to the inline one. It is
//! compiled to a shared library and co-loaded alongside the package's own library;
//! the generated cdef declares the `pnl_si_*` symbols and the method dispatches to
//! them, so the inline functions become ordinary bound methods.
//!
//! The C compiler does all the semantic work — pnl never reimplements C. The only
//! parsing here is splitting a declaration into return type / name / parameters so a
//! forwarding call can be written; a declaration we cannot confidently forward is
//! skipped and stays a throwing stub.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cc::CCompiler;
use crate::header_adapter::UnsupportedFunction;
use crate::manifest::{DefinitionType, ResolvedDefinition};

/// Symbol prefix for the exported trampolines.
const SHIM_PREFIX: &str = "pnl_si_";

/// Inputs for building a shim, threaded from the install flow.
pub struct ShimRequest {
    /// The located compiler, or `None` (then [`build`] errors if there is anything
    /// to shim — `compile_options.static_inline` was enabled without a compiler).
    pub compiler: Option<CCompiler>,
    /// Directory the shim `.c` source and compiled library are written into.
    pub out_dir: PathBuf,
    /// File stem (per library key), e.g. `example_example`.
    pub stem: String,
    /// The package's resolved header files to `#include`.
    pub headers: Vec<PathBuf>,
    /// `-I` directories (the same set the libclang parse used).
    pub include_dirs: Vec<PathBuf>,
    /// `-D` definitions (resolved `require_definitions`).
    pub definitions: Vec<ResolvedDefinition>,
    /// The resolved primary library to link against, so the inline bodies' calls
    /// into it resolve regardless of co-load order. Empty for a virtual/system lib.
    pub primary_library: String,
    /// Package name, for diagnostics.
    pub package: String,
}

/// One shimmed function: the cdef declaration to add and its public name.
#[derive(Debug)]
pub struct ShimEntry {
    /// The original (public) C function name, e.g. `example_inline_double`.
    pub public_name: String,
    /// The `pnl_si_<name>(...)` declaration to append to the cdef so FFI binds it.
    pub cdef_declaration: String,
    /// The original `static inline` declaration this replaces (matched against the
    /// `unsupported_functions` list so the throwing stub is dropped).
    pub original_declaration: String,
}

/// A built shim: the compiled library plus the functions it exports.
#[derive(Debug)]
pub struct BuiltShim {
    pub library: PathBuf,
    pub entries: Vec<ShimEntry>,
}

/// The result of attempting a shim build, so the caller can distinguish a fatal
/// configuration error (no compiler) from a recoverable per-package one (the shim
/// did not compile), which falls back to throwing stubs.
#[derive(Debug)]
pub enum ShimOutcome {
    /// A shim was built for these functions.
    Built(BuiltShim),
    /// Nothing forwardable (e.g. only variadic inlines); no compiler needed.
    Empty,
    /// Functions to shim exist but no C compiler was found — fatal, since the
    /// consumer explicitly enabled `compile_options.static_inline`.
    MissingCompiler { functions: Vec<String> },
    /// A compiler is present but the shim translation unit did not compile (e.g. a
    /// header needs a transitive include libclang tolerated dropping). Recoverable:
    /// the functions stay throwing stubs, exactly as with the option off.
    CompileFailed {
        functions: Vec<String>,
        detail: String,
    },
}

/// Build a trampoline shim for the given `static inline` functions.
pub fn build(request: &ShimRequest, static_inlines: &[UnsupportedFunction]) -> ShimOutcome {
    let mut entries = Vec::new();
    let mut definitions = Vec::new();
    for function in static_inlines {
        let Some(tramp) = trampoline(&function.declaration) else {
            continue;
        };
        definitions.push(tramp.definition);
        entries.push(ShimEntry {
            public_name: tramp.name,
            cdef_declaration: tramp.cdef_declaration,
            original_declaration: function.declaration.clone(),
        });
    }
    if entries.is_empty() {
        return ShimOutcome::Empty;
    }

    let names = || {
        entries
            .iter()
            .map(|entry| entry.public_name.clone())
            .collect()
    };
    let Some(compiler) = request.compiler.as_ref() else {
        return ShimOutcome::MissingCompiler { functions: names() };
    };
    match compile(request, compiler, &definitions) {
        Ok(library) => ShimOutcome::Built(BuiltShim { library, entries }),
        Err(error) => ShimOutcome::CompileFailed {
            functions: names(),
            detail: error.to_string(),
        },
    }
}

/// Compile the trampoline definitions into a shared library next to the package.
fn compile(request: &ShimRequest, compiler: &CCompiler, definitions: &[String]) -> Result<PathBuf> {
    std::fs::create_dir_all(&request.out_dir)
        .with_context(|| format!("failed to create shim dir {}", request.out_dir.display()))?;

    let mut source = String::new();
    source.push_str(
        "/* Auto-generated by pnl. Trampolines exporting `static inline` functions. */\n",
    );
    source.push_str("/* !!! DO NOT EDIT THIS FILE !!! */\n");
    for header in &request.headers {
        // Absolute, so the `#include` resolves from the deep `shim/` dir the file
        // lives in (the install passes project-root-relative header paths).
        source.push_str(&format!("#include \"{}\"\n", absolute(header).display()));
    }
    source.push('\n');
    // Wrap the trampolines in `extern "C"` so a C++ retry keeps their symbols
    // unmangled; the guard is inert under a plain C compile.
    source.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for definition in definitions {
        source.push_str(definition);
        source.push('\n');
    }
    source.push_str("#ifdef __cplusplus\n}\n#endif\n");

    let c_path = request.out_dir.join(format!("{}_shim.c", request.stem));
    std::fs::write(&c_path, &source)
        .with_context(|| format!("failed to write {}", c_path.display()))?;

    let extension = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let lib_path = request
        .out_dir
        .join(format!("lib{}_shim.{extension}", request.stem));

    // Compile as C first; if that fails, retry as C++. Some libraries expose a C API
    // whose headers only compile as C++ (assimp: bare `aiMaterial` needs the `struct`
    // tag in C). On a double failure the C error is the more representative one.
    let c = invoke(request, compiler, &c_path, &lib_path, None)?;
    if c.status.success() {
        return Ok(lib_path);
    }
    let cpp = invoke(request, compiler, &c_path, &lib_path, Some("c++"))?;
    if cpp.status.success() {
        return Ok(lib_path);
    }
    bail!(
        "failed to compile static-inline shim for {}:\n{}",
        request.package,
        String::from_utf8_lossy(&c.stderr).trim()
    );
}

/// Run the compiler once over the shim source, optionally forcing a `-x <lang>`
/// (e.g. `c++`). Returns the process output; a non-zero status is a compile failure,
/// not an error here (the caller decides whether to retry).
fn invoke(
    request: &ShimRequest,
    compiler: &CCompiler,
    c_path: &Path,
    lib_path: &Path,
    language: Option<&str>,
) -> Result<std::process::Output> {
    let mut command = std::process::Command::new(&compiler.program);
    command.args(&compiler.cflags);
    // Mirror the libclang parse: the macOS SDK sysroot and the same `-I`/`-D` set so
    // the inline bodies (and their config macros, e.g. pcre2 width) compile.
    command.args(crate::header_adapter::macos_isysroot_args());
    for dir in &request.include_dirs {
        if let Some(dir) = absolute(dir).to_str() {
            command.arg(format!("-I{dir}"));
        }
    }
    for definition in &request.definitions {
        command.arg(define_arg(definition));
    }
    command.arg("-fPIC").arg("-shared");
    if cfg!(target_os = "macos") {
        // The inline bodies may reference the primary library's exported symbols,
        // resolved at load/call time from the global table where it is co-loaded.
        command.arg("-Wl,-undefined,dynamic_lookup");
    }
    command.arg("-o").arg(lib_path);
    // `-x <lang>` applies to every following input, so scope it to the shim source
    // only and reset to `-x none` before the linked library (else the compiler tries
    // to parse the `.dylib`/`.so` as C++ source).
    if let Some(language) = language {
        command.arg("-x").arg(language);
    }
    command.arg(c_path);
    if language.is_some() {
        command.arg("-x").arg("none");
    }
    // Link the primary library directly when it is a real file, so its symbols
    // resolve no matter the co-load order (eager linkers like musl bind on dlopen).
    let primary = Path::new(&request.primary_library);
    if !request.primary_library.is_empty() && primary.is_file() {
        command.arg(&request.primary_library);
    }
    command.args(&compiler.ldflags);

    command
        .output()
        .with_context(|| format!("failed to run C compiler {}", compiler.program))
}

/// An absolute form of `path` (canonicalized when it exists), so the compiled shim
/// is independent of the compiler's working directory. Falls back to the original.
fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Render one resolved definition as a `-D` flag, mirroring the libclang parse
/// (string values keep their quotes so the macro expands to a C string literal).
fn define_arg(definition: &ResolvedDefinition) -> String {
    match definition.definition_type {
        DefinitionType::String => format!("-D{}=\"{}\"", definition.name, definition.value),
        _ => format!("-D{}={}", definition.name, definition.value),
    }
}

/// A parsed forwarding trampoline.
struct Trampoline {
    name: String,
    cdef_declaration: String,
    definition: String,
}

/// Build a forwarding trampoline for a `ret name(params);` declaration, or `None`
/// when the parameters cannot be forwarded (function-less prototype, varargs, an
/// unnamed parameter) — the caller leaves such a function a throwing stub.
fn trampoline(declaration: &str) -> Option<Trampoline> {
    let (return_type, name, params) = split_signature(declaration)?;
    let args = forward_args(&params)?;
    let shim_name = format!("{SHIM_PREFIX}{name}");
    let params_sig = if params.trim().is_empty() {
        "void".to_owned()
    } else {
        params.clone()
    };
    let cdef_declaration = format!("{return_type} {shim_name}({params_sig});");
    let call = format!("{name}({})", args.join(", "));
    let body = if return_type.trim() == "void" {
        format!("{call};")
    } else {
        format!("return {call};")
    };
    let definition = format!("{return_type} {shim_name}({params_sig}) {{ {body} }}");
    Some(Trampoline {
        name,
        cdef_declaration,
        definition,
    })
}

/// Split `ret name(params);` into `(return type, name, params)`. The parameter list
/// is the outermost parentheses ending at the final `)`; the name is the identifier
/// immediately before it.
fn split_signature(declaration: &str) -> Option<(String, String, String)> {
    let declaration = declaration.trim().trim_end_matches(';').trim();
    let close = declaration.rfind(')')?;
    let mut depth = 0i32;
    let mut open = None;
    for (index, ch) in declaration[..=close].char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let open = open?;
    let params = declaration[open + 1..close].trim().to_owned();
    let head = declaration[..open].trim();
    let name_start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map(|index| index + 1)
        .unwrap_or(0);
    let name = head[name_start..].to_owned();
    let return_type = head[..name_start].trim().to_owned();
    if name.is_empty() || return_type.is_empty() {
        return None;
    }
    Some((return_type, name, params))
}

/// The argument names to forward for a parameter list, or `None` if any parameter
/// has no usable name (so the function cannot be forwarded).
fn forward_args(params: &str) -> Option<Vec<String>> {
    let trimmed = params.trim();
    if trimmed.is_empty() || trimmed == "void" {
        return Some(Vec::new());
    }
    let mut args = Vec::new();
    for param in split_params(params) {
        args.push(param_name(&param)?);
    }
    Some(args)
}

/// Split a parameter list on top-level commas (respecting parentheses/brackets so a
/// function-pointer or array parameter is not split).
fn split_params(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(params[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    let last = params[start..].trim();
    if !last.is_empty() {
        out.push(last.to_owned());
    }
    out
}

/// Extract the declared identifier from a single parameter declaration, or `None`
/// for varargs (`...`) or an unnamed parameter.
fn param_name(param: &str) -> Option<String> {
    let param = param.trim();
    if param.is_empty() || param == "void" || param == "..." {
        return None;
    }
    // Function pointer: `ret (*name)(args)` — the name follows `(*`.
    if let Some(star) = param.find("(*") {
        let name: String = param[star + 2..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // Otherwise the name is the trailing identifier (after stripping any `[...]`).
    let head = param.split('[').next().unwrap_or(param).trim_end();
    let start = head
        .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map(|index| index + 1)
        .unwrap_or(0);
    let name = &head[start..];
    // A bare type with no name (e.g. `int`) leaves a type keyword as the trailing
    // token; treat that as unnamed so the function is not mis-forwarded.
    if name.is_empty() || start == 0 || is_type_keyword(name) {
        return None;
    }
    Some(name.to_owned())
}

/// Whether a trailing token is a C type/qualifier keyword rather than a parameter
/// name (so a type-only, unnamed parameter is detected).
fn is_type_keyword(token: &str) -> bool {
    matches!(
        token,
        "void"
            | "char"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "const"
            | "volatile"
            | "struct"
            | "union"
            | "enum"
            | "bool"
            | "_Bool"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_a_simple_inline() {
        let tramp = trampoline("double example_inline_double(double x);").expect("parsed");
        assert_eq!(tramp.name, "example_inline_double");
        assert_eq!(
            tramp.cdef_declaration,
            "double pnl_si_example_inline_double(double x);"
        );
        assert_eq!(
            tramp.definition,
            "double pnl_si_example_inline_double(double x) { return example_inline_double(x); }"
        );
    }

    #[test]
    fn forwards_void_return_and_no_params() {
        let tramp = trampoline("void do_thing(void);").expect("parsed");
        assert_eq!(
            tramp.definition,
            "void pnl_si_do_thing(void) { do_thing(); }"
        );
    }

    #[test]
    fn forwards_pointer_and_multiple_params() {
        let tramp = trampoline("int add(const char *name, int count);").expect("parsed");
        assert_eq!(
            tramp.definition,
            "int pnl_si_add(const char *name, int count) { return add(name, count); }"
        );
    }

    #[test]
    fn forwards_function_pointer_param() {
        let tramp = trampoline("int run(int (*cb)(int), void *ctx);").expect("parsed");
        assert_eq!(
            tramp.definition,
            "int pnl_si_run(int (*cb)(int), void *ctx) { return run(cb, ctx); }"
        );
    }

    #[test]
    fn skips_variadic() {
        assert!(trampoline("int logf(const char *fmt, ...);").is_none());
    }

    fn request_without_compiler() -> ShimRequest {
        ShimRequest {
            compiler: None,
            out_dir: std::env::temp_dir().join("pnl-shim-test"),
            stem: "x".to_owned(),
            headers: Vec::new(),
            include_dirs: Vec::new(),
            definitions: Vec::new(),
            primary_library: String::new(),
            package: "vendor/pkg".to_owned(),
        }
    }

    fn static_inline(declaration: &str) -> UnsupportedFunction {
        UnsupportedFunction {
            declaration: declaration.to_owned(),
            reason: "static inline".to_owned(),
        }
    }

    #[test]
    fn reports_missing_compiler_when_a_shim_is_needed() {
        let inlines = vec![static_inline("int f(int x);")];
        match build(&request_without_compiler(), &inlines) {
            ShimOutcome::MissingCompiler { functions } => assert_eq!(functions, vec!["f"]),
            other => panic!("expected MissingCompiler, got {other:?}"),
        }
    }

    #[test]
    fn empty_when_nothing_can_be_shimmed() {
        // A variadic prototype cannot be forwarded, so there is nothing to compile
        // and no compiler is required.
        let inlines = vec![static_inline("int f(const char *a, ...);")];
        assert!(matches!(
            build(&request_without_compiler(), &inlines),
            ShimOutcome::Empty
        ));
    }
}
