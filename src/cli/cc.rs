//! C compiler discovery for compiled trampoline shims (see [`crate::shim`]).
//!
//! pnl's only hard install-time requirement is libclang; a C compiler is needed
//! solely for the opt-in `compile_options.static_inline` shim. Discovery honours the
//! universal `CC`/`CFLAGS`/`LDFLAGS` convention (make, autotools, the `cc` crate),
//! then probes `cc` → `clang` → `gcc` on `$PATH`.

use std::path::Path;

/// A located C compiler plus the environment-provided flags to honour.
#[derive(Debug, Clone)]
pub struct CCompiler {
    /// The compiler program to invoke (`$CC`, else `cc`/`clang`/`gcc`).
    pub program: String,
    /// Extra flags from `$CFLAGS`.
    pub cflags: Vec<String>,
    /// Extra flags from `$LDFLAGS`.
    pub ldflags: Vec<String>,
}

/// Probe order, mirrored in [`compiler_not_found_message`] and the docs.
const PROBE_ORDER: &[&str] = &["cc", "clang", "gcc"];

/// Find a usable C compiler: `$CC` if set and resolvable, else the first of
/// `cc`/`clang`/`gcc` found on `$PATH`. Honours `$CFLAGS`/`$LDFLAGS`. Returns `None`
/// when none is available.
pub fn find_c_compiler() -> Option<CCompiler> {
    let program = std::env::var("CC")
        .ok()
        .map(|cc| cc.trim().to_owned())
        .filter(|cc| !cc.is_empty() && program_exists(cc))
        .or_else(|| {
            PROBE_ORDER
                .iter()
                .find(|candidate| program_exists(candidate))
                .map(|candidate| (*candidate).to_owned())
        })?;
    Some(CCompiler {
        program,
        cflags: split_env("CFLAGS"),
        ldflags: split_env("LDFLAGS"),
    })
}

/// Whether `program` resolves to a file: a path component is checked directly, a
/// bare name is searched along `$PATH`.
fn program_exists(program: &str) -> bool {
    if program.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(program).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

/// Whitespace-split the named environment variable into individual flags.
fn split_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The actionable error shown when a shim is required (a package has `static inline`
/// functions and `compile_options.static_inline` is on) but no compiler was found.
pub fn compiler_not_found_message(package: &str, functions: &[String]) -> String {
    format!(
        "compile_options.static_inline is enabled, but building a trampoline shim for \
         {package}'s `static inline` function(s) needs a C compiler and none was found.\n  \
         functions: {functions}\n  \
         looked for: $CC, then cc, clang, gcc on $PATH.\n  \
         Install a C compiler (macOS: `xcode-select --install`; Linux: your distro's \
         clang/gcc package) or set CC=/path/to/cc, then reinstall.\n  \
         Or turn it off with `pnl config compile_options.static_inline false` — the \
         functions stay throwing stubs.",
        functions = functions.join(", "),
    )
}
