use std::collections::BTreeMap;
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
        });
    }

    let names = &requirement.library_names;
    let mut dirs = Vec::new();
    dirs.extend(manifest.load_paths.iter().map(PathBuf::from));
    dirs.extend(env_path_dirs("DYLD_LIBRARY_PATH"));
    dirs.extend(env_path_dirs("LD_LIBRARY_PATH"));
    dirs.extend(env_path_dirs("PATH"));
    dirs.extend([
        PathBuf::from("/opt/homebrew/lib"),
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/lib"),
    ]);

    let searched = dedupe_paths(dirs)
        .into_iter()
        .map(|dir| absolutize(root, &dir))
        .collect::<Vec<_>>();

    // Prefer a real on-disk library file (skipping virtual entries).
    for dir in &searched {
        for name in names.iter().filter(|name| !name.is_virtual()) {
            let path = dir.join(name.name());
            if path.is_file() {
                let version =
                    pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
                validate_semver(&version)?;
                return Ok(ResolvedNativeLibrary {
                    resolved_name: name.name().to_owned(),
                    path: path.display().to_string(),
                    version,
                    sha256: sha256_file(&path)?,
                });
            }
        }
    }

    // Fall back to a virtual (system) library: linked by name, never required to
    // exist as a file (e.g. libc, which on macOS lives in the dyld shared cache).
    if let Some(name) = names.iter().find(|name| name.is_virtual()) {
        let version = pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
        validate_semver(&version)?;
        return Ok(ResolvedNativeLibrary {
            resolved_name: name.name().to_owned(),
            // A bare file name (no directory) signals a system library to the
            // bridge linker, which then links by name without an -L path.
            path: name.name().to_owned(),
            version,
            sha256: sha256_hex(name.name().as_bytes()),
        });
    }

    bail!(
        "{}",
        native_library_not_found_message(key, names, &searched)
    );
}

fn native_library_not_found_message(
    key: &str,
    names: &[LibraryName],
    searched: &[PathBuf],
) -> String {
    let stem = key_without_version(key);
    // Many library keys already start with `lib` (e.g. `libpq`); avoid suggesting
    // `liblibpq-dev`.
    let dev_package = if stem.starts_with("lib") {
        format!("{stem}-dev")
    } else {
        format!("lib{stem}-dev")
    };
    let mut message = format!("could not find the native library for \"{key}\".\n\n");
    let name_list = names
        .iter()
        .map(LibraryName::name)
        .collect::<Vec<_>>()
        .join(", ");
    message.push_str(&format!("  looked for: {name_list}\n"));
    message.push_str("  in:\n");
    for dir in searched {
        message.push_str(&format!("    - {}\n", dir.display()));
    }
    message.push_str(&format!(
        "\nInstall the library and make it discoverable, e.g.:\n  \
         - brew install {stem}   (macOS)\n  \
         - apt-get install {dev_package}   (Debian/Ubuntu)\n  \
         - add its directory to \"load_paths\" in pnl.json\n  \
         - or export DYLD_LIBRARY_PATH / LD_LIBRARY_PATH to point at it\n\n\
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

fn pkg_config_version(key: &str) -> Option<String> {
    let output = ProcessCommand::new("pkg-config")
        .arg("--modversion")
        .arg(key)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn pkg_config_include_dirs(key: &str) -> Vec<PathBuf> {
    let output = ProcessCommand::new("pkg-config")
        .arg("--cflags-only-I")
        .arg(key)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|flag| flag.strip_prefix("-I"))
        .map(PathBuf::from)
        .collect()
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeMap::new();
    for path in paths {
        seen.entry(path.display().to_string()).or_insert(path);
    }
    seen.into_values().collect()
}
