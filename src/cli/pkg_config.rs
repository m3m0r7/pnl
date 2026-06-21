//! A minimal, self-contained `pkg-config`.
//!
//! pnl needs three things from pkg-config when resolving a native library: the
//! module version, its `-I` include directories (with `Requires:` merged in, so a
//! library's transitive devel headers — e.g. GLib's `glibconfig.h` behind pango —
//! are seen), and its `-L` library directory. All of that lives in the library's
//! `.pc` file, which ships in the same `-dev`/`-devel` package (or Homebrew
//! formula) as the headers pnl already requires. Parsing those files directly means
//! pnl does not depend on the external `pkg-config`/`pkgconf` binary being
//! installed — only on the `.pc` files themselves, which are present whenever
//! pkg-config would have had anything to report.
//!
//! This is intentionally not a full pkg-config: it covers variable expansion,
//! `Requires`/`Requires.private` transitive merge for `--cflags`, and the standard
//! search-path rules (`PKG_CONFIG_PATH`/`PKG_CONFIG_LIBDIR` plus system and
//! Homebrew defaults). Comparison operators in `Requires:` are ignored (only the
//! module names matter for directory discovery).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The version reported by the first of `modules` whose `.pc` file is found.
pub(crate) fn modversion(modules: &[String]) -> Option<String> {
    let dirs = search_dirs();
    for module in modules {
        if let Some(pc) = PcFile::find(module, &dirs) {
            // `Version:` is frequently written as a `${version}` variable reference
            // (OpenBLAS, ncurses), so it must be expanded like any other field.
            if let Some(version) = pc.field_expanded("Version") {
                let version = version.trim();
                if !version.is_empty() {
                    return Some(version.to_owned());
                }
            }
        }
    }
    None
}

/// The `-I` include directories for `modules`, with `Requires`/`Requires.private`
/// merged transitively (matching `pkg-config --cflags-only-I`).
pub(crate) fn include_dirs(modules: &[String]) -> Vec<PathBuf> {
    let dirs = search_dirs();
    let mut out = Vec::new();
    let mut visited = BTreeSet::new();
    for module in modules {
        collect_cflag_includes(module, &dirs, &mut visited, &mut out);
    }
    out
}

/// The `-L` library directories declared in `modules`' own `Libs:`
/// (matching `pkg-config --libs-only-L` for the named modules).
pub(crate) fn lib_dirs(modules: &[String]) -> Vec<PathBuf> {
    let dirs = search_dirs();
    let mut out = Vec::new();
    for module in modules {
        if let Some(pc) = PcFile::find(module, &dirs) {
            for dir in flag_dirs(&pc.field_expanded("Libs").unwrap_or_default(), "-L") {
                push_unique(&mut out, dir);
            }
        }
    }
    out
}

/// Recursively gather a module's own `-I` dirs plus those of everything it
/// `Requires` (public and private — both contribute to compilation flags).
fn collect_cflag_includes(
    module: &str,
    dirs: &[PathBuf],
    visited: &mut BTreeSet<String>,
    out: &mut Vec<PathBuf>,
) {
    if !visited.insert(module.to_owned()) {
        return;
    }
    let Some(pc) = PcFile::find(module, dirs) else {
        return;
    };
    for dir in flag_dirs(&pc.field_expanded("Cflags").unwrap_or_default(), "-I") {
        push_unique(out, dir);
    }
    for required in pc.requires() {
        collect_cflag_includes(&required, dirs, visited, out);
    }
}

/// A parsed `.pc` file: its `name=value` variables and `Field: value` properties.
struct PcFile {
    variables: BTreeMap<String, String>,
    fields: BTreeMap<String, String>,
}

impl PcFile {
    /// Locate `<module>.pc` in the search directories and parse it.
    fn find(module: &str, dirs: &[PathBuf]) -> Option<PcFile> {
        let file_name = format!("{module}.pc");
        for dir in dirs {
            let path = dir.join(&file_name);
            if path.is_file()
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                return Some(PcFile::parse(&text));
            }
        }
        None
    }

    fn parse(text: &str) -> PcFile {
        let mut variables = BTreeMap::new();
        let mut fields = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // A property is `Keyword: value`; a variable is `name=value`. They are
            // told apart by whichever delimiter appears first (a `Cflags:` value may
            // contain `=`, a `prefix=` value rarely contains `:` before its `=`).
            let colon = line.find(':');
            let equals = line.find('=');
            match (colon, equals) {
                (Some(c), eq) if eq.is_none_or(|e| c < e) => {
                    let (key, value) = line.split_at(c);
                    fields.insert(key.trim().to_owned(), value[1..].trim().to_owned());
                }
                (_, Some(e)) => {
                    let (key, value) = line.split_at(e);
                    variables.insert(key.trim().to_owned(), value[1..].trim().to_owned());
                }
                _ => {}
            }
        }
        PcFile { variables, fields }
    }

    /// A raw property value (no variable expansion).
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// A property value with `${var}` references expanded.
    fn field_expanded(&self, name: &str) -> Option<String> {
        self.field(name)
            .map(|value| expand(value, &self.variables, 0))
    }

    /// The module names this `.pc` `Requires`/`Requires.private`, with any version
    /// constraints (`glib-2.0 >= 2.40`) stripped.
    fn requires(&self) -> Vec<String> {
        let mut modules = Vec::new();
        for key in ["Requires", "Requires.private"] {
            let Some(value) = self.field_expanded(key) else {
                continue;
            };
            let mut tokens = value
                .split(|ch: char| ch.is_whitespace() || ch == ',')
                .filter(|token| !token.is_empty())
                .peekable();
            while let Some(token) = tokens.next() {
                // `name (>= ver)?` — a bare comparison operator or version number is
                // not a module name.
                if is_version_operator(token) {
                    tokens.next(); // consume the version token that follows
                    continue;
                }
                if token.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                modules.push(token.to_owned());
            }
        }
        modules
    }
}

/// Expand `${name}` references against `variables`, guarding against cycles.
fn expand(value: &str, variables: &BTreeMap<String, String>, depth: usize) -> String {
    if depth > 32 || !value.contains("${") {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match variables.get(name) {
            Some(replacement) => out.push_str(&expand(replacement, variables, depth + 1)),
            None => {
                out.push_str("${");
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Extract the directories carried by a compile/link flag (`-I`/`-L`) from a
/// flag string, supporting both `-Idir` and `-I dir` spellings.
fn flag_dirs(flags: &str, prefix: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut tokens = flags.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let dir = if token == prefix {
            tokens.next().map(str::to_owned)
        } else {
            token.strip_prefix(prefix).map(str::to_owned)
        };
        if let Some(dir) = dir.filter(|dir| !dir.is_empty()) {
            dirs.push(PathBuf::from(dir));
        }
    }
    dirs
}

fn is_version_operator(token: &str) -> bool {
    matches!(token, "=" | "==" | "!=" | "<" | ">" | "<=" | ">=")
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// The directories searched for `.pc` files, mirroring pkg-config: an explicit
/// `PKG_CONFIG_LIBDIR` replaces the system defaults, `PKG_CONFIG_PATH` prepends to
/// them, and Homebrew prefixes (including keg-only `opt/*` cellars) are added so
/// macOS works without a system pkg-config.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = std::env::var_os("PKG_CONFIG_PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    match std::env::var_os("PKG_CONFIG_LIBDIR") {
        Some(libdir) => dirs.extend(std::env::split_paths(&libdir)),
        None => dirs.extend(default_search_dirs()),
    }
    dedupe(dirs)
}

fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // The active Homebrew prefix comes FIRST — matching how Homebrew's own
    // pkg-config is configured — so a current keg wins over a stale system `.pc`
    // (e.g. macOS ships an old `/usr/lib/pkgconfig/libpcre2-8.pc`). Apple-silicon
    // uses `/opt/homebrew`, Intel `/usr/local`; the other is kept as a fallback.
    let brew_prefixes: &[&str] = if cfg!(target_arch = "aarch64") {
        &["/opt/homebrew", "/usr/local"]
    } else {
        &["/usr/local", "/opt/homebrew"]
    };
    for prefix in brew_prefixes {
        dirs.push(PathBuf::from(prefix).join("lib/pkgconfig"));
        dirs.push(PathBuf::from(prefix).join("share/pkgconfig"));
        // Keg-only formulae are not symlinked into the prefix but reachable via `opt/`.
        dirs.extend(opt_pkgconfig_dirs(Path::new(prefix)));
        dirs.extend(child_pkgconfig_dirs(&PathBuf::from(prefix).join("lib")));
    }
    // Standard system locations, including 64-bit and Debian/Ubuntu multiarch libdirs.
    for base in ["/usr/lib", "/usr/lib64", "/usr/share"] {
        dirs.push(PathBuf::from(base).join("pkgconfig"));
    }
    dirs.extend(child_pkgconfig_dirs(Path::new("/usr/lib")));
    dirs
}

/// `<base>/<entry>/pkgconfig` for each immediate child of `base` (multiarch).
fn child_pkgconfig_dirs(base: &Path) -> Vec<PathBuf> {
    read_dir_children(base)
        .into_iter()
        .map(|child| child.join("pkgconfig"))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// `<prefix>/opt/<formula>/lib/pkgconfig` for each keg-only Homebrew formula.
fn opt_pkgconfig_dirs(prefix: &Path) -> Vec<PathBuf> {
    read_dir_children(&prefix.join("opt"))
        .into_iter()
        .map(|formula| formula.join("lib/pkgconfig"))
        .filter(|dir| dir.is_dir())
        .collect()
}

fn read_dir_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect()
}

fn dedupe(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variables_and_fields_with_expansion() {
        let pc = PcFile::parse(
            "prefix=/opt/pango\n\
             version=1.54.0\n\
             libdir=${prefix}/lib\n\
             includedir=${prefix}/include\n\
             Name: Pango\n\
             Version: ${version}\n\
             Requires: glib-2.0 >= 2.40, harfbuzz\n\
             Cflags: -I${includedir}/pango-1.0\n\
             Libs: -L${libdir} -lpango-1.0\n",
        );
        // `Version:` written as a variable reference is expanded (OpenBLAS/ncurses).
        assert_eq!(pc.field_expanded("Version").as_deref(), Some("1.54.0"));
        assert_eq!(
            pc.field_expanded("Cflags").as_deref(),
            Some("-I/opt/pango/include/pango-1.0")
        );
        // The version constraint is stripped; both required modules survive.
        assert_eq!(pc.requires(), vec!["glib-2.0", "harfbuzz"]);
    }

    #[test]
    fn extracts_dirs_from_both_flag_spellings() {
        assert_eq!(
            flag_dirs("-I/a -I /b -DFOO=bar -I/c", "-I"),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
        assert_eq!(flag_dirs("-L/x -lpango", "-L"), vec![PathBuf::from("/x")]);
    }

    #[test]
    fn expansion_leaves_unknown_variables_and_breaks_cycles() {
        let mut vars = BTreeMap::new();
        vars.insert("a".to_owned(), "${b}".to_owned());
        vars.insert("b".to_owned(), "${a}".to_owned());
        // A cycle terminates instead of looping forever.
        let _ = expand("${a}", &vars, 0);
        // An undefined reference is left verbatim.
        assert_eq!(expand("x${missing}y", &BTreeMap::new(), 0), "x${missing}y");
    }
}
