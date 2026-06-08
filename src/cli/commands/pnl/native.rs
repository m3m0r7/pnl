use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Result, bail};

use crate::manifest::{PnlManifest, ResolvedHeader, ResolvedNativeLibrary};
use crate::validate::validate_semver;

use super::package::{absolutize, sha256_file};

pub(super) fn resolve_native_library(
    root: &Path,
    manifest: &PnlManifest,
    key: &str,
    names: &[String],
) -> Result<ResolvedNativeLibrary> {
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

    for dir in dedupe_paths(dirs) {
        let dir = absolutize(root, &dir);
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                let version =
                    pkg_config_version(key).unwrap_or_else(|| native_version_from_key(key));
                validate_semver(&version)?;
                return Ok(ResolvedNativeLibrary {
                    resolved_name: name.clone(),
                    path: path.display().to_string(),
                    version,
                    sha256: sha256_file(&path)?,
                });
            }
        }
    }

    bail!("could not resolve native library {key}");
}

pub(super) fn resolve_header_for_native(
    extension_root: &Path,
    native_path: &str,
    key: &str,
    header_names: &[String],
) -> Result<ResolvedHeader> {
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

    for root in dedupe_paths(include_roots) {
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

    bail!("could not resolve header for native library {key}");
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
