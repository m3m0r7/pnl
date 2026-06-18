use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};

use crate::fetch::fetch_asset;
use crate::manifest::{
    LibraryName, NativeRequirement, PnlManifest, ResolvedHeader, ResolvedNativeLibrary,
};
use crate::validate::validate_semver;

use super::package::{absolutize, sha256_file, sha256_hex};

pub(super) fn resolve_native_library(
    root: &Path,
    manifest: &PnlManifest,
    key: &str,
    requirement: &NativeRequirement,
) -> Result<ResolvedNativeLibrary> {
    if let Some(url) = &requirement.library_url {
        let path = fetch_asset(url)
            .with_context(|| format!("failed to fetch native library for {key} from {url}"))?;
        let resolved_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(key)
            .to_owned();
        let version = pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
        validate_semver(&version)?;
        return Ok(ResolvedNativeLibrary {
            resolved_name,
            path: path.display().to_string(),
            version,
            sha256: sha256_file(&path)?,
            installed_at: None,
        });
    }

    let names = &requirement.library_names;
    let mut dirs = Vec::new();
    dirs.extend(manifest.load_paths.iter().map(PathBuf::from));
    dirs.extend(env_path_dirs("DYLD_LIBRARY_PATH"));
    dirs.extend(env_path_dirs("LD_LIBRARY_PATH"));
    dirs.extend(env_path_dirs("PATH"));
    // Ask pkg-config where the library actually lives — this is what makes
    // resolution work across distributions and architectures (e.g. Debian/Ubuntu
    // install shared objects under a multiarch dir like
    // `/usr/lib/x86_64-linux-gnu`, which the fixed list below does not cover).
    dirs.extend(pkg_config_lib_dirs(key));
    dirs.extend([
        PathBuf::from("/opt/homebrew/lib"),
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/lib"),
    ]);
    // Debian/Ubuntu multiarch directories, for libraries that ship no
    // pkg-config `.pc` file (so the lookup above finds nothing).
    for triplet in multiarch_triplets() {
        dirs.push(PathBuf::from(format!("/usr/lib/{triplet}")));
        dirs.push(PathBuf::from(format!("/lib/{triplet}")));
    }

    let searched = dedupe_paths(dirs)
        .into_iter()
        .map(|dir| absolutize(root, &dir))
        .collect::<Vec<_>>();

    // Prefer a real on-disk library file (skipping virtual entries). The
    // unversioned `libfoo.so` is a development symlink that only exists when the
    // `-dev`/`-devel` package is installed; fall back to a versioned soname
    // (`libfoo.so.N`), which is what is present on a runtime-only system and is
    // also what PHP FFI ultimately dlopen()s.
    for dir in &searched {
        for name in names.iter().filter(|name| !name.is_virtual()) {
            if let Some(path) = find_library_file(dir, name.name()) {
                let version =
                    pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
                validate_semver(&version)?;
                let resolved_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(name.name())
                    .to_owned();
                return Ok(ResolvedNativeLibrary {
                    resolved_name,
                    path: path.display().to_string(),
                    version,
                    sha256: sha256_file(&path)?,
                    installed_at: None,
                });
            }
        }
    }

    // Fall back to a virtual (system) library: linked by name, never required to
    // exist as a file (e.g. libc, which on macOS lives in the dyld shared cache).
    // Prefer the name whose extension matches the current OS so that, e.g.,
    // `libc.so.6` is chosen on Linux rather than the first-listed `libc.dylib`.
    let virtual_name = names
        .iter()
        .find(|name| name.is_virtual() && library_name_matches_os(name.name()))
        .or_else(|| names.iter().find(|name| name.is_virtual()));
    if let Some(name) = virtual_name {
        let version = pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
        validate_semver(&version)?;
        return Ok(ResolvedNativeLibrary {
            resolved_name: name.name().to_owned(),
            // A bare file name (no directory) signals a virtual/system library
            // that PHP FFI should open by name.
            path: name.name().to_owned(),
            version,
            sha256: sha256_hex(name.name().as_bytes()),
            installed_at: None,
        });
    }

    bail!(
        "{}",
        native_library_not_found_message(key, names, &requirement.header_names, &searched)
    );
}

/// Locate a library file in `dir`: the exact `base` name if present, otherwise
/// a versioned soname `base.N[.N...]` (e.g. `libgmp.so` -> `libgmp.so.10`).
/// Among versioned matches the shortest name wins, which is the soname symlink
/// (`libfoo.so.2`) rather than the fully-qualified file (`libfoo.so.2.0.9`).
fn find_library_file(dir: &Path, base: &str) -> Option<PathBuf> {
    let exact = dir.join(base);
    if exact.is_file() {
        return Some(exact);
    }
    let prefix = format!("{base}.");
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let suffix = name.strip_prefix(&prefix)?;
            // Only a version suffix (digits and dots), so `libfoo.so` matches
            // `libfoo.so.2` but never `libfoo.so.conf` or `libfoobar.so`.
            (!suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit() || ch == '.'))
                .then(|| entry.path())
        })
        .collect();
    matches.sort_by_key(|path| path.as_os_str().len());
    matches.into_iter().next()
}

/// The function/data symbols a shared library actually exports, used to drop
/// cdef declarations the installed library does not provide (PHP FFI resolves
/// every declared symbol eagerly, so one missing symbol fails the whole load —
/// common with version/build skew, e.g. `sqlite3_mutex_held` in a non-debug
/// SQLite, or a newer `BrotliEncoderPrepareDictionary`).
///
/// Returns `None` (meaning "do not filter") for a virtual/bare-name library or
/// when the symbol table cannot be read, preserving the previous behaviour.
pub(super) fn exported_symbols(library_path: &str) -> Option<BTreeSet<String>> {
    // A bare file name signals a virtual/system library with no resolved path.
    if !library_path.contains(std::path::MAIN_SEPARATOR) {
        return None;
    }
    // `nm -D` lists dynamic symbols on Linux; `nm -gU` is the macOS spelling.
    for args in [&["-D", "--defined-only"][..], &["-gU"][..]] {
        let Ok(output) = ProcessCommand::new("nm")
            .args(args)
            .arg(library_path)
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let symbols: BTreeSet<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next_back()?;
                // Lines with only a name (undefined) have no type column; require
                // at least a type letter before the name.
                if line.split_whitespace().count() < 2 {
                    return None;
                }
                // Strip ELF symbol-versioning suffixes (`compressBound@@ZLIB_1.2.0`).
                let name = name.split('@').next().unwrap_or(name);
                // Mach-O prefixes every C symbol with a single leading underscore;
                // strip exactly that one on macOS. ELF (Linux) has no such prefix, so
                // keep the name verbatim — `trim_start_matches('_')` would corrupt
                // legitimately underscore-prefixed symbols like GMP's `__gmpz_init`.
                let name = if cfg!(target_os = "macos") {
                    name.strip_prefix('_').unwrap_or(name)
                } else {
                    name
                };
                Some(name.to_owned())
            })
            .collect();
        if !symbols.is_empty() {
            return Some(symbols);
        }
    }
    None
}

/// Whether a library file name matches the current OS's shared-object naming
/// (`.so` on Linux, `.dylib` on macOS, `.dll` on Windows). Used to pick the
/// right virtual/system library when several OS variants are listed.
fn library_name_matches_os(name: &str) -> bool {
    match std::env::consts::OS {
        "macos" => name.contains(".dylib"),
        "windows" => name.ends_with(".dll"),
        _ => name.contains(".so"),
    }
}

fn native_library_not_found_message(
    key: &str,
    names: &[LibraryName],
    header_names: &[String],
    searched: &[PathBuf],
) -> String {
    let stem = key_without_version(key);
    // Many library keys already start with `lib` (e.g. `libfoo`); avoid suggesting
    // `liblibfoo-dev`.
    let dev_package = if stem.starts_with("lib") {
        format!("{stem}-dev")
    } else {
        format!("lib{stem}-dev")
    };
    let name_list = names
        .iter()
        .map(LibraryName::name)
        .collect::<Vec<_>>()
        .join(", ");
    let header_list = if header_names.is_empty() {
        "(none declared)".to_owned()
    } else {
        header_names.join(", ")
    };

    // This package does not declare an `installation` recipe, so the user must
    // provide the native library and headers themselves.
    let mut message = format!(
        "could not find the native library for \"{key}\".\n\n\
         This package has no \"installation\" commands, so you must place the\n\
         library and its headers somewhere on your search path yourself.\n\n\
         pnlx is looking for:\n  \
         - library : {name_list}\n  \
         - headers : {header_list}\n\n\
         It searched these directories:\n"
    );
    for dir in searched {
        message.push_str(&format!("    - {}\n", dir.display()));
    }
    message.push_str(&format!(
        "\nMake them discoverable, e.g.:\n  \
         - brew install {stem}   (macOS)\n  \
         - apt-get install {dev_package}   (Debian/Ubuntu)\n  \
         - or copy the files into a directory that is already on your path,\n    \
         e.g. the library into /usr/local/lib and the headers into /usr/local/include\n  \
         - or add their directory to \"load_paths\" in pnl.json, or export\n    \
         DYLD_LIBRARY_PATH / LD_LIBRARY_PATH\n\n\
         If the file name differs, set \"library_names\" for \"{key}\" in pnlx.json."
    ));
    message
}

/// Write an inline header into the installed package's generated directory and
/// return its path.
fn write_inline_header(installed_root: &Path, key: &str, contents: &str) -> Result<PathBuf> {
    let dir = installed_root.join("src").join("generated");
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let file_name = format!("{}.h", key.replace(['/', '\\'], "_"));
    let path = dir.join(file_name);
    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

pub(super) fn resolve_header_for_native(
    extension_root: &Path,
    installed_root: &Path,
    native_path: &str,
    key: &str,
    requirement: &NativeRequirement,
) -> Result<ResolvedHeader> {
    if let Some(contents) = &requirement.header_inline {
        // Inline headers belong to the installed package (so they live and die
        // with it), not a shared user cache.
        let path = write_inline_header(installed_root, key, contents)
            .with_context(|| format!("failed to write inline header for {key}"))?;
        return Ok(ResolvedHeader {
            sha256: sha256_file(&path)?,
            path: path.display().to_string(),
        });
    }

    if let Some(url) = &requirement.header_url {
        let path = fetch_asset(url)
            .with_context(|| format!("failed to fetch header for {key} from {url}"))?;
        return Ok(ResolvedHeader {
            sha256: sha256_file(&path)?,
            path: path.display().to_string(),
        });
    }

    let header_names = &requirement.header_names;
    let native_path = Path::new(native_path);
    let mut include_roots = Vec::new();
    include_roots.extend(pkg_config_include_dirs(key));
    include_roots.extend(env_path_dirs("CPATH"));
    include_roots.extend(env_path_dirs("C_INCLUDE_PATH"));
    include_roots.extend(env_path_dirs("PATH"));

    if let Some(prefix) = native_path.parent().and_then(Path::parent) {
        include_roots.push(prefix.join("include"));
    }
    include_roots.push(extension_root.join("include"));
    include_roots.push(extension_root.to_path_buf());
    include_roots.extend([
        PathBuf::from("/opt/homebrew/include"),
        PathBuf::from("/usr/local/include"),
        PathBuf::from("/usr/include"),
    ]);
    // Debian/Ubuntu install architecture-specific headers (e.g. `ffi.h`,
    // `gmp.h`, `tiffio.h`) under a multiarch include dir, not plain
    // `/usr/include`.
    for triplet in multiarch_triplets() {
        include_roots.push(PathBuf::from(format!("/usr/include/{triplet}")));
    }

    let roots = dedupe_paths(include_roots);
    for root in &roots {
        for relative in header_candidates(key, header_names) {
            let path = root.join(&relative);
            if path.is_file() {
                return Ok(ResolvedHeader {
                    path: path.display().to_string(),
                    sha256: sha256_file(&path)?,
                });
            }
        }
    }

    let mut message = format!("could not find a C header for \"{key}\".\n\n");
    message.push_str(&format!(
        "  looked for: {}\n  in:\n",
        header_candidates(key, header_names)
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    for root in &roots {
        message.push_str(&format!("    - {}\n", root.display()));
    }
    message.push_str(&format!(
        "\nInstall the library's development headers (e.g. the -dev/-devel package), \
         or set \"header_names\" for \"{key}\" in pnlx.json, \
         or add the include directory to CPATH."
    ));
    bail!("{message}");
}

pub(super) fn generation_headers_from_resolved_header(
    header: &ResolvedHeader,
    header_names: &[String],
) -> Vec<PathBuf> {
    let primary = PathBuf::from(&header.path);
    let mut headers = vec![primary.clone()];

    if let Some(include_root) = include_root_from_resolved_header(&primary, header_names) {
        for name in header_names {
            let candidate = include_root.join(name);
            if candidate.is_file() {
                headers.push(candidate);
            }
        }
    }

    headers.sort();
    headers.dedup();
    headers
}

pub(super) fn key_without_version(key: &str) -> String {
    key.rsplit_once('-')
        .and_then(|(stem, suffix)| {
            suffix
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == '.')
                .then(|| stem.to_owned())
        })
        .unwrap_or_else(|| key.to_owned())
}

fn include_root_from_resolved_header(path: &Path, header_names: &[String]) -> Option<PathBuf> {
    for name in header_names {
        let suffix = Path::new(name);
        if !path.ends_with(suffix) {
            continue;
        }

        let mut root = path;
        for _ in suffix.components() {
            root = root.parent()?;
        }

        return Some(root.to_path_buf());
    }

    None
}

fn header_candidates(key: &str, header_names: &[String]) -> Vec<PathBuf> {
    let stem = key_without_version(key);
    let mut candidates = header_names.iter().map(PathBuf::from).collect::<Vec<_>>();
    candidates.extend([
        PathBuf::from(key).join(format!("{stem}.h")),
        PathBuf::from(format!("{key}.h")),
        PathBuf::from(format!("{stem}.h")),
        PathBuf::from(key).join(format!("{key}.h")),
    ]);
    candidates
}

fn native_version_from_key(key: &str) -> String {
    key.rsplit_once('-')
        .map(|(_, version)| version)
        .filter(|version| version.chars().any(|ch| ch.is_ascii_digit()))
        .map(|version| {
            let mut parts = version.split('.').collect::<Vec<_>>();
            while parts.len() < 3 {
                parts.push("0");
            }
            parts[..3].join(".")
        })
        .unwrap_or_else(|| "0.0.0".to_owned())
}

fn env_path_dirs(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

/// pkg-config module names to try for a requirement key. The key is rarely the
/// exact `.pc` module name (e.g. key `libpixman-1` -> module `pixman-1`, key
/// `libdbus-1` -> module `dbus-1`), so also try the key without its leading
/// `lib`.
fn pkg_config_modules(key: &str) -> Vec<String> {
    let mut modules = vec![key.to_owned()];
    if let Some(stripped) = key.strip_prefix("lib")
        && !stripped.is_empty()
    {
        modules.push(stripped.to_owned());
    }
    modules
}

fn pkg_config_version(key: &str) -> Option<String> {
    for module in pkg_config_modules(key) {
        let Ok(output) = ProcessCommand::new("pkg-config")
            .arg("--modversion")
            .arg(&module)
            .output()
        else {
            continue;
        };
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
    }
    None
}

fn pkg_config_include_dirs(key: &str) -> Vec<PathBuf> {
    pkg_config_flag_dirs(key, "--cflags-only-I", "-I")
}

/// Run `pkg-config <flag>` for every candidate module and collect the directory
/// arguments (those carrying `prefix`, e.g. `-I` or `-L`) across all of them.
fn pkg_config_flag_dirs(key: &str, flag: &str, prefix: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for module in pkg_config_modules(key) {
        let Ok(output) = ProcessCommand::new("pkg-config")
            .arg(flag)
            .arg(&module)
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for dir in String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .filter_map(|flag| flag.strip_prefix(prefix))
            .map(PathBuf::from)
        {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Library directories reported by pkg-config (the `-L` flags). On many systems
/// the only non-default `-L` is the architecture-specific libdir, which is
/// exactly the directory the plain `/usr/lib` fallback misses.
fn pkg_config_lib_dirs(key: &str) -> Vec<PathBuf> {
    pkg_config_flag_dirs(key, "--libs-only-L", "-L")
}

/// The compiler's multiarch tuple(s) (e.g. `x86_64-linux-gnu`), used to build
/// Debian/Ubuntu library paths like `/usr/lib/x86_64-linux-gnu`. Empty on
/// platforms whose compiler does not implement `-print-multiarch`.
fn multiarch_triplets() -> Vec<String> {
    let mut triplets = Vec::new();
    for cc in ["cc", "gcc", "clang"] {
        if let Ok(output) = ProcessCommand::new(cc).arg("-print-multiarch").output()
            && output.status.success()
        {
            let triplet = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !triplet.is_empty() && !triplets.contains(&triplet) {
                triplets.push(triplet);
            }
        }
    }
    triplets
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeMap::new();
    for path in paths {
        seen.entry(path.display().to_string()).or_insert(path);
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkg_config_modules_also_tries_without_lib_prefix() {
        assert_eq!(
            pkg_config_modules("libpixman-1"),
            ["libpixman-1", "pixman-1"]
        );
        assert_eq!(pkg_config_modules("libdbus-1"), ["libdbus-1", "dbus-1"]);
        // No leading `lib`: only the key itself.
        assert_eq!(pkg_config_modules("openssl"), ["openssl"]);
    }

    #[test]
    fn find_library_file_prefers_exact_then_shortest_soname() {
        let dir = tempfile::tempdir().unwrap();
        let touch = |name: &str| std::fs::write(dir.path().join(name), b"x").unwrap();

        // Only a versioned soname is present (the unversioned dev symlink is not).
        touch("libgmp.so.10");
        touch("libgmp.so.10.5.0");
        let found = find_library_file(dir.path(), "libgmp.so").unwrap();
        assert_eq!(found.file_name().unwrap(), "libgmp.so.10");

        // An exact match wins over versioned ones.
        touch("libgmp.so");
        let found = find_library_file(dir.path(), "libgmp.so").unwrap();
        assert_eq!(found.file_name().unwrap(), "libgmp.so");
    }

    #[test]
    fn find_library_file_ignores_non_version_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("libfoo.so.conf"), b"x").unwrap();
        std::fs::write(dir.path().join("libfoobar.so"), b"x").unwrap();
        assert!(find_library_file(dir.path(), "libfoo.so").is_none());
    }
}
