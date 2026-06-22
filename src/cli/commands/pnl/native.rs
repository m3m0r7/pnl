use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};

use crate::fetch::fetch_asset;
use crate::manifest::{
    LibraryName, LoadType, NativeRequirement, PnlManifest, ResolvedHeader, ResolvedNativeLibrary,
};
use crate::validate::{normalize_semver, validate_semver};

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
            export_source: None,
            co_load: BTreeMap::new(),
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
    // macOS keeps no on-disk system dylibs (dyld shared cache); the active SDK ships
    // `.tbd` text stubs declaring their exports. Adding the SDK lib dir makes those
    // discoverable like any file, so a `libSystem.B.tbd` route resolves there.
    dirs.extend(macos_sdk_lib_dirs());

    let searched = dedupe_paths(dirs)
        .into_iter()
        .map(|dir| absolutize(root, &dir))
        .collect::<Vec<_>>();

    // `library_names` is an ORDERED FALLBACK CHAIN: try each route in turn and take
    // the first that resolves. A route resolves to a real on-disk file when found —
    // INCLUDING for a `virtual` entry: "virtual" means "no file is REQUIRED", not
    // "ignore the file if present", so the export filter still runs (dropping a
    // declared-but-unexported symbol like glibc's `atexit`). A `virtual` route that
    // matches the OS otherwise resolves by name (the dyld-cache fallback), so the
    // chain can terminate in a system library that need not exist on disk.
    for name in names {
        if let Some(resolved) = resolve_route_file(name, &searched, key)? {
            return Ok(resolved);
        }
        if name.is_virtual() && library_name_matches_os(name.name()) {
            return virtual_route(name, key);
        }
    }
    // Last resort: any virtual route, even if its extension doesn't match the OS.
    if let Some(name) = names.iter().find(|name| name.is_virtual()) {
        return virtual_route(name, key);
    }

    bail!(
        "{}",
        native_library_not_found_message(key, names, &requirement.header_names, &searched)
    );
}

/// Resolve one route to a real on-disk file (searching `dirs`), or `None` if no
/// file is found. A `.tbd` stub (macOS SDK) is read for its exports while the
/// runtime load target becomes the stub's `install-name` — the `.tbd` itself is
/// never `dlopen`ed.
fn resolve_route_file(
    name: &LibraryName,
    dirs: &[PathBuf],
    key: &str,
) -> Result<Option<ResolvedNativeLibrary>> {
    let Some(file) = dirs
        .iter()
        .find_map(|dir| find_library_file(dir, name.name()))
    else {
        return Ok(None);
    };
    let version = pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
    validate_semver(&version)?;
    let resolved_name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name.name())
        .to_owned();
    let is_tbd = matches!(name.load_type(), LoadType::Tbd)
        || (name.load_type() == LoadType::Auto && file.extension().is_some_and(|ext| ext == "tbd"));
    if is_tbd {
        let content = std::fs::read_to_string(&file)
            .with_context(|| format!("failed to read tbd stub {}", file.display()))?;
        let tbd = crate::tbd::parse_tbd(&content)
            .with_context(|| format!("{} is not a parseable .tbd stub", file.display()))?;
        // Load target = the stub's install-name (dlopen resolves it from the dyld
        // cache); exports are read from the `.tbd` file via `export_source`.
        let load_path = tbd
            .install_name
            .unwrap_or_else(|| file.display().to_string());
        return Ok(Some(ResolvedNativeLibrary {
            resolved_name,
            path: load_path,
            export_source: Some(file.display().to_string()),
            co_load: BTreeMap::new(),
            version,
            sha256: sha256_file(&file)?,
            installed_at: None,
        }));
    }
    Ok(Some(ResolvedNativeLibrary {
        resolved_name,
        path: file.display().to_string(),
        export_source: None,
        // A GNU ld linker script at the unversioned dev name (`libncurses.so` =
        // `INPUT(libncurses.so.6 -ltinfo)`) means the real symbols are split across
        // several `.so`s; co-load the extras (libtinfo) so their symbols resolve.
        co_load: linker_script_co_load(dirs, name.name(), &file),
        version,
        sha256: sha256_file(&file)?,
        installed_at: None,
    }))
}

/// When the unversioned dev name (`<base>`, e.g. `libncurses.so`) is a GNU ld
/// linker script, parse its `INPUT(...)`/`GROUP(...)` for the libraries it pulls in
/// and resolve each to a real path, EXCLUDING the already-resolved primary. The
/// result is the extra shared objects to co-load (`name -> path`). Empty when there
/// is no linker script (the common case, and every non-ELF platform).
fn linker_script_co_load(dirs: &[PathBuf], base: &str, primary: &Path) -> BTreeMap<String, String> {
    let mut co_load = BTreeMap::new();
    let Some(script_path) = dirs
        .iter()
        .map(|dir| dir.join(base))
        .find(|path| path.is_file())
    else {
        return co_load;
    };
    let Ok(content) = std::fs::read_to_string(&script_path) else {
        return co_load;
    };
    let primary_name = primary.file_name().and_then(|n| n.to_str());
    for lib_name in parse_linker_script_inputs(&content) {
        if let Some(file) = dirs
            .iter()
            .find_map(|dir| find_library_file(dir, &lib_name))
        {
            let resolved_name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&lib_name)
                .to_owned();
            // Skip the primary itself (the script lists it alongside its extras).
            if Some(resolved_name.as_str()) == primary_name {
                continue;
            }
            co_load.insert(resolved_name, file.display().to_string());
        }
    }
    co_load
}

/// Library file names referenced by a GNU ld linker script's `INPUT(...)` /
/// `GROUP(...)` / `AS_NEEDED(...)` directives: a bare `libfoo.so.N` token is taken
/// verbatim, and a `-lfoo` token becomes `libfoo.so` (resolved to its real soname
/// by [`find_library_file`]). Returns an empty list for a non-linker-script file.
fn parse_linker_script_inputs(content: &str) -> Vec<String> {
    // Only treat ASCII text that actually uses the directives as a linker script.
    if !content.contains("INPUT") && !content.contains("GROUP") {
        return Vec::new();
    }
    let mut names = Vec::new();
    // Flatten the directive arguments: replace separators with spaces and read the
    // tokens. `INPUT ( a b )`, `GROUP(a,b)`, `AS_NEEDED(...)` all reduce to tokens.
    let flattened: String = content
        .chars()
        .map(|ch| match ch {
            '(' | ')' | ',' => ' ',
            other => other,
        })
        .collect();
    for token in flattened.split_whitespace() {
        if matches!(token, "INPUT" | "GROUP" | "OUTPUT_FORMAT" | "AS_NEEDED") {
            continue;
        }
        if let Some(stem) = token.strip_prefix("-l") {
            if !stem.is_empty() {
                names.push(format!("lib{stem}.so"));
            }
        } else if token.contains(".so") && !token.contains('/') {
            // A bare shared-object name (e.g. `libncurses.so.6`); ignore absolute
            // paths and the OUTPUT_FORMAT(...) ELF-flavour tokens.
            names.push(token.to_owned());
        }
    }
    names
}

/// A virtual/system library route: linked by name, never required to exist as a
/// file (libc on macOS lives in the dyld shared cache). No export filter runs (no
/// file to read), so the chain should keep such routes last.
fn virtual_route(name: &LibraryName, key: &str) -> Result<ResolvedNativeLibrary> {
    let version = pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
    validate_semver(&version)?;
    Ok(ResolvedNativeLibrary {
        resolved_name: name.name().to_owned(),
        // A bare file name (no directory) signals a virtual/system library that PHP
        // FFI should open by name.
        path: name.name().to_owned(),
        export_source: None,
        co_load: BTreeMap::new(),
        version,
        sha256: sha256_hex(name.name().as_bytes()),
        installed_at: None,
    })
}

/// The active macOS SDK's `usr/lib` directory (where `.tbd` stubs live), via
/// `xcrun --show-sdk-path` with an `xcode-select -p` fallback. Empty off macOS.
fn macos_sdk_lib_dirs() -> Vec<PathBuf> {
    if std::env::consts::OS != "macos" {
        return Vec::new();
    }
    let run = |program: &str, args: &[&str]| -> Option<String> {
        let output = ProcessCommand::new(program).args(args).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    if let Some(sdk) = run("xcrun", &["--show-sdk-path"]).filter(|s| !s.is_empty()) {
        return vec![PathBuf::from(sdk).join("usr/lib")];
    }
    // Fallback: derive the default macOS SDK under the developer dir.
    if let Some(dev) = run("xcode-select", &["-p"]).filter(|s| !s.is_empty()) {
        return vec![
            PathBuf::from(&dev).join("Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/usr/lib"),
            PathBuf::from(&dev).join("SDKs/MacOSX.sdk/usr/lib"),
        ];
    }
    Vec::new()
}

/// Locate a library file in `dir`: the exact `base` name if present, otherwise
/// a versioned soname `base.N[.N...]` (e.g. `libgmp.so` -> `libgmp.so.10`).
/// Among versioned matches the shortest name wins, which is the soname symlink
/// (`libfoo.so.2`) rather than the fully-qualified file (`libfoo.so.2.0.9`).
fn find_library_file(dir: &Path, base: &str) -> Option<PathBuf> {
    let exact = dir.join(base);
    // Accept the exact name only if it is something the runtime can actually load:
    // a real shared object, or a `.tbd` stub. A bare `libfoo.so` is often a GNU ld
    // linker script (a tiny text file, `INPUT(libfoo.so.6 …)`) which `FFI` can't
    // `dlopen` ("file too short"); skip it and fall through to the versioned soname
    // `libfoo.so.N`, which is the real object (and what dlopen resolves at runtime).
    if exact.is_file() && is_loadable_library_file(&exact) {
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

/// Whether a file is something the runtime can load as a native library: a real
/// shared object (ELF / Mach-O / PE, detected by magic) or a macOS `.tbd` text
/// stub. A GNU ld linker script (ASCII text that is not a `.tbd`) is rejected so
/// resolution falls through to the real versioned soname.
fn is_loadable_library_file(path: &Path) -> bool {
    if path.extension().is_some_and(|ext| ext == "tbd") {
        return true;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        // Can't introspect it — be permissive (don't reject a real library just
        // because we couldn't open it for a peek).
        return true;
    };
    use std::io::Read;
    let mut magic = [0u8; 4];
    let Ok(read) = file.read(&mut magic) else {
        return true;
    };
    let magic = &magic[..read];
    magic.starts_with(b"\x7fELF")            // ELF (Linux)
        || magic.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) // Mach-O 64 LE
        || magic.starts_with(&[0xce, 0xfa, 0xed, 0xfe]) // Mach-O 32 LE
        || magic.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) // Mach-O universal (fat)
        || magic.starts_with(b"MZ") // PE (Windows .dll)
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
    // Parse the shared library's export table directly (ELF `.dynsym` defined-global
    // symbols, the Mach-O export trie, the PE export directory) instead of shelling
    // out to `nm`: the export filter must always work so a declared-but-unexported
    // symbol (e.g. SDL's app-provided `SDL_main`) is dropped, and `pnl install`'s
    // only external toolchain requirement stays libclang — not binutils.
    let data = std::fs::read(library_path).ok()?;
    // A `.tbd` text stub (macOS SDK) isn't a Mach-O binary — read its declared
    // exports through the YAML-based parser instead of `object`.
    if library_path.ends_with(".tbd") {
        return crate::tbd::parse_tbd(&String::from_utf8_lossy(&data))
            .map(|tbd| tbd.symbols)
            .filter(|symbols| !symbols.is_empty());
    }
    let file = object::File::parse(&*data).ok()?;
    let exports = object::Object::exports(&file).ok()?;
    let symbols: BTreeSet<String> = exports
        .iter()
        .filter_map(|export| {
            let name = std::str::from_utf8(export.name()).ok()?;
            // Strip an ELF symbol-versioning suffix (`compressBound@@ZLIB_1.2.0`).
            let name = name.split('@').next().unwrap_or(name);
            // Mach-O prefixes every C symbol with a single leading underscore; strip
            // exactly that one on macOS. ELF (Linux) has no such prefix, so keep the
            // name verbatim — stripping all `_` would corrupt legitimately
            // underscore-prefixed symbols like GMP's `__gmpz_init`.
            let name = if cfg!(target_os = "macos") {
                name.strip_prefix('_').unwrap_or(name)
            } else {
                name
            };
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect();
    (!symbols.is_empty()).then_some(symbols)
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
            include_dirs: Vec::new(),
        });
    }

    if let Some(url) = &requirement.header_url {
        let path = fetch_asset(url)
            .with_context(|| format!("failed to fetch header for {key} from {url}"))?;
        return Ok(ResolvedHeader {
            sha256: sha256_file(&path)?,
            path: path.display().to_string(),
            include_dirs: Vec::new(),
        });
    }

    let header_names = &requirement.header_names;
    let native_path = Path::new(native_path);
    // The compiler's own include flags for this library — recorded so the cdef parse
    // sees libdir devel headers (GLib's `glibconfig.h`/`pango-features.h`).
    let pkg_config_dirs = pkg_config_include_dirs(key);
    let mut include_roots = Vec::new();
    include_roots.extend(pkg_config_dirs.iter().cloned());
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
                    include_dirs: pkg_config_dirs
                        .iter()
                        .filter_map(|dir| dir.to_str().map(str::to_owned))
                        .collect(),
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
/// exact `.pc` module name, so also try variants: the key without its leading
/// `lib` (key `libpixman-1` -> module `pixman-1`, key `libdbus-1` -> module
/// `dbus-1`), and the key WITH a `lib` prefix (key `enet` -> module `libenet`,
/// whose `.pc` is `libenet.pc`).
fn pkg_config_modules(key: &str) -> Vec<String> {
    let mut modules = vec![key.to_owned()];
    if let Some(stripped) = key.strip_prefix("lib")
        && !stripped.is_empty()
    {
        modules.push(stripped.to_owned());
    } else {
        modules.push(format!("lib{key}"));
    }
    modules
}

fn pkg_config_version(key: &str) -> Option<String> {
    crate::pkg_config::modversion(&pkg_config_modules(key)).map(|version| {
        // pkg-config `Version:` fields are not constrained to three components
        // (z3 reports `4.15.4.0`), but the lockfile schema and the `semver` crate
        // expect a canonical `major.minor.patch`. Canonicalize here so every
        // resolution path records a lock-safe version; fall back to the raw value
        // if it cannot be parsed, letting the later `validate_semver` surface it.
        normalize_semver(&version)
            .map(|canonical| canonical.to_string())
            .unwrap_or(version)
    })
}

/// The `-I` include directories pkg-config would report for this library (with
/// `Requires:` merged), parsed from the `.pc` files directly so pnl needs no
/// external pkg-config binary.
fn pkg_config_include_dirs(key: &str) -> Vec<PathBuf> {
    crate::pkg_config::include_dirs(&pkg_config_modules(key))
}

/// Library directories declared by the library's `.pc` (the `-L` flags). On many
/// systems the only non-default `-L` is the architecture-specific libdir, which is
/// exactly the directory the plain `/usr/lib` fallback misses.
fn pkg_config_lib_dirs(key: &str) -> Vec<PathBuf> {
    crate::pkg_config::lib_dirs(&pkg_config_modules(key))
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
    fn pkg_config_modules_tries_lib_prefix_variants() {
        // A `lib`-prefixed key also tries the stripped form.
        assert_eq!(
            pkg_config_modules("libpixman-1"),
            ["libpixman-1", "pixman-1"]
        );
        assert_eq!(pkg_config_modules("libdbus-1"), ["libdbus-1", "dbus-1"]);
        // A key without a leading `lib` also tries the `lib`-prefixed form
        // (key `enet` -> module `libenet`, whose `.pc` is `libenet.pc`).
        assert_eq!(pkg_config_modules("enet"), ["enet", "libenet"]);
        assert_eq!(pkg_config_modules("openssl"), ["openssl", "libopenssl"]);
    }

    #[test]
    fn find_library_file_prefers_exact_then_shortest_soname() {
        let dir = tempfile::tempdir().unwrap();
        // Real libraries carry ELF magic so they pass the loadable-object check.
        let elf = |name: &str| std::fs::write(dir.path().join(name), b"\x7fELFreal").unwrap();

        // Only a versioned soname is present (the unversioned dev symlink is not).
        elf("libgmp.so.10");
        elf("libgmp.so.10.5.0");
        let found = find_library_file(dir.path(), "libgmp.so").unwrap();
        assert_eq!(found.file_name().unwrap(), "libgmp.so.10");

        // A real exact match wins over versioned ones.
        elf("libgmp.so");
        let found = find_library_file(dir.path(), "libgmp.so").unwrap();
        assert_eq!(found.file_name().unwrap(), "libgmp.so");
    }

    #[test]
    fn parses_linker_script_inputs_for_coload() {
        // ncurses's dev linker script: the primary plus `-ltinfo`. INPUT tokens are
        // surfaced (bare soname verbatim, `-l` → `lib*.so`); a plain file is not.
        assert_eq!(
            parse_linker_script_inputs("/* GNU ld script */\nINPUT(libncurses.so.6 -ltinfo)\n"),
            vec!["libncurses.so.6".to_owned(), "libtinfo.so".to_owned()]
        );
        // GROUP(...) and AS_NEEDED(...) are handled the same way.
        assert_eq!(
            parse_linker_script_inputs("GROUP ( libc.so.6 AS_NEEDED ( -lpthread ) )"),
            vec!["libc.so.6".to_owned(), "libpthread.so".to_owned()]
        );
        // A non-linker-script file yields nothing.
        assert!(parse_linker_script_inputs("\x7fELF...binary...").is_empty());
        assert!(parse_linker_script_inputs("just some text").is_empty());
    }

    #[test]
    fn linker_script_co_load_excludes_primary_and_resolves_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let elf = |name: &str| std::fs::write(dir.path().join(name), b"\x7fELFreal").unwrap();
        // The dev symlink is a linker script naming the primary + libtinfo.
        std::fs::write(
            dir.path().join("libncurses.so"),
            b"INPUT(libncurses.so.6 -ltinfo)\n",
        )
        .unwrap();
        elf("libncurses.so.6");
        elf("libtinfo.so.6");
        let primary = dir.path().join("libncurses.so.6");
        let co = linker_script_co_load(&[dir.path().to_path_buf()], "libncurses.so", &primary);
        // libtinfo is co-loaded; the primary itself is excluded.
        assert_eq!(co.len(), 1);
        assert!(co.contains_key("libtinfo.so.6"));
        assert!(!co.contains_key("libncurses.so.6"));
    }

    #[test]
    fn find_library_file_skips_linker_script_for_real_soname() {
        let dir = tempfile::tempdir().unwrap();
        // `libncurses.so` is a GNU ld linker script (ASCII text, no ELF magic); the
        // real object is the versioned soname. Resolution must skip the script.
        std::fs::write(
            dir.path().join("libncurses.so"),
            b"/* GNU ld script */\nINPUT(libncurses.so.6 -ltinfo)\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("libncurses.so.6"), b"\x7fELFreal").unwrap();
        let found = find_library_file(dir.path(), "libncurses.so").unwrap();
        assert_eq!(found.file_name().unwrap(), "libncurses.so.6");
    }

    #[test]
    fn find_library_file_ignores_non_version_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("libfoo.so.conf"), b"x").unwrap();
        std::fs::write(dir.path().join("libfoobar.so"), b"x").unwrap();
        assert!(find_library_file(dir.path(), "libfoo.so").is_none());
    }
}
