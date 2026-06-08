use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};

use crate::io::{read_json, write_json};
use crate::manifest::{PnlLock, PnlxManifest, ResolvedBridge, ResolvedNativeLibrary};
use crate::platform::now;
use crate::validate::{
    ensure_platform_matches, sanitize_package_segment, validate_pnlx_manifest_values,
    validate_schema_version,
};

use super::native::key_without_version;
use super::package::{
    installed_extension_dir, json_string, pnl_lock_path, pnlx_pathmap_path,
    read_pathmap_for_current_platform, relative_to_root, sha256_file,
};

pub(crate) fn build_installed_bridges(root: &Path, packages: &[String]) -> Result<()> {
    let lock_path = pnl_lock_path(root);
    if !lock_path.exists() {
        bail!("no @pnlx/pnlx-lock.json found; run pnl install before pnlx build");
    }

    let lock = read_json::<PnlLock>(&lock_path)?;
    ensure_platform_matches(&lock.platform)?;
    let mut pathmap = read_pathmap_for_current_platform(root)?;
    let packages = installed_bridge_packages(&lock, packages)?;
    let mut built = 0usize;

    for package in packages {
        let extension_root = installed_extension_dir(root, &package);
        let manifest_path = extension_root.join("pnlx.json");
        if !manifest_path.is_file() {
            bail!(
                "installed extension {} is missing {}; run pnl install {} again",
                package,
                manifest_path.display(),
                package
            );
        }

        let extension = read_json::<PnlxManifest>(&manifest_path)?;
        validate_schema_version(&extension.schema_version)?;
        validate_pnlx_manifest_values(&extension)?;

        for key in extension.requires.keys() {
            let native = pathmap
                .requires
                .get(key)
                .with_context(|| {
                    format!(
                        "native library {key} is not resolved in @pnlx/pnlx-pathmap.json; run pnl install {} first",
                        extension.name
                    )
                })?
                .clone();
            if let Some(bridge) = compile_bridge_for_library(root, &extension_root, key, &native)? {
                pathmap.bridges.insert(key.clone(), bridge);
                built += 1;
            }
        }
    }

    pathmap.generated_at = now();
    write_json(&pnlx_pathmap_path(root), &pathmap)?;
    println!("built {built} bridge(s)");
    Ok(())
}

pub(super) fn compile_bridge_for_library(
    root: &Path,
    extension_root: &Path,
    library_key: &str,
    native: &ResolvedNativeLibrary,
) -> Result<Option<ResolvedBridge>> {
    let bridge_source = match resolve_bridge_source(extension_root, library_key)? {
        Some(path) => path,
        None => return Ok(None),
    };
    let bridge_dir = root
        .join("@pnlx")
        .join("bridges")
        .join(sanitize_package_segment(library_key)?);
    if bridge_dir.exists() {
        fs::remove_dir_all(&bridge_dir)
            .with_context(|| format!("failed to remove {}", bridge_dir.display()))?;
    }
    fs::create_dir_all(&bridge_dir)
        .with_context(|| format!("failed to create {}", bridge_dir.display()))?;

    let source_name = bridge_source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bridge.rs");
    let installed_source = bridge_dir.join(source_name);
    fs::copy(&bridge_source, &installed_source).with_context(|| {
        format!(
            "failed to copy {} to {}",
            bridge_source.display(),
            installed_source.display()
        )
    })?;

    let library = compile_bridge(root, library_key, native, &bridge_dir, &installed_source)?;
    Ok(Some(ResolvedBridge {
        source: relative_to_root(root, &installed_source),
        library: relative_to_root(root, &library),
        sha256: sha256_file(&library)?,
    }))
}

fn compile_bridge(
    root: &Path,
    library_key: &str,
    native: &ResolvedNativeLibrary,
    bridge_dir: &Path,
    installed_source: &Path,
) -> Result<PathBuf> {
    let crate_source = bridge_dir.join("crate.rs");
    let include_source = fs::canonicalize(installed_source)
        .with_context(|| format!("failed to resolve {}", installed_source.display()))?;
    fs::write(
        &crate_source,
        format!("include!({});\n", json_string(&include_source)),
    )
    .with_context(|| format!("failed to write {}", crate_source.display()))?;

    let library = bridge_dir.join(bridge_library_file(library_key));
    let native_path = Path::new(&native.path);
    let native_dir = native_path
        .parent()
        .with_context(|| format!("native library has no parent directory: {}", native.path))?;
    let mut command = ProcessCommand::new("rustc");
    command
        .arg("--crate-type")
        .arg("cdylib")
        .arg(&crate_source)
        .arg("-L")
        .arg(format!("native={}", native_dir.display()))
        .arg("-l")
        .arg(format!("dylib={}", native_link_name(native_path)));
    add_native_rpath(&mut command, native_dir);
    let status = command
        .arg("-o")
        .arg(&library)
        .current_dir(root)
        .status()
        .with_context(|| "failed to start rustc for bridge compilation")?;
    if !status.success() {
        bail!(
            "failed to compile bridge {} for {}",
            installed_source.display(),
            library_key
        );
    }
    Ok(library)
}

fn installed_bridge_packages(lock: &PnlLock, packages: &[String]) -> Result<Vec<String>> {
    if packages.is_empty() {
        return Ok(lock.extensions.keys().cloned().collect());
    }

    let mut resolved = Vec::new();
    for package in packages {
        let package = resolve_installed_bridge_package(lock, package)?;
        if !resolved.contains(&package) {
            resolved.push(package);
        }
    }
    Ok(resolved)
}

fn resolve_installed_bridge_package(lock: &PnlLock, package: &str) -> Result<String> {
    if lock.extensions.contains_key(package) {
        return Ok(package.to_owned());
    }

    let matches = lock
        .extensions
        .keys()
        .filter(|name| {
            name.rsplit_once('/')
                .is_some_and(|(_, leaf)| leaf == package)
        })
        .cloned()
        .collect::<Vec<_>>();

    match matches.len() {
        0 => bail!("{package} is not installed"),
        1 => Ok(matches.into_iter().next().expect("len checked")),
        _ => bail!("package name {package} is ambiguous; use vendor/package"),
    }
}

fn resolve_bridge_source(extension_root: &Path, library_key: &str) -> Result<Option<PathBuf>> {
    let generated_dir = extension_root.join("src/generated");
    if !generated_dir.is_dir() {
        return Ok(None);
    }

    let preferred = generated_dir.join(format!(
        "{}.bridge.rs",
        sanitize_artifact_stem(&key_without_version(library_key))
    ));
    if preferred.is_file() {
        return Ok(Some(preferred));
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(&generated_dir)
        .with_context(|| format!("failed to read {}", generated_dir.display()))?
    {
        let path = entry
            .with_context(|| format!("failed to read {}", generated_dir.display()))?
            .path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".bridge.rs"))
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    Ok(candidates.into_iter().next())
}

fn bridge_library_file(library_key: &str) -> String {
    let stem = format!(
        "{}_bridge",
        sanitize_artifact_stem(&key_without_version(library_key)).replace('-', "_")
    );
    match std::env::consts::OS {
        "macos" => format!("{}.dylib", unix_library_stem(&stem)),
        "windows" => format!("{stem}.dll"),
        _ => format!("{}.so", unix_library_stem(&stem)),
    }
}

fn unix_library_stem(stem: &str) -> String {
    if stem.starts_with("lib") {
        stem.to_owned()
    } else {
        format!("lib{stem}")
    }
}

fn native_link_name(path: &Path) -> String {
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let without_prefix = file.strip_prefix("lib").unwrap_or(file);
    without_prefix
        .strip_suffix(".dylib")
        .or_else(|| without_prefix.strip_suffix(".so"))
        .or_else(|| without_prefix.strip_suffix(".dll"))
        .unwrap_or(without_prefix)
        .to_owned()
}

fn add_native_rpath(command: &mut ProcessCommand, native_dir: &Path) {
    if matches!(std::env::consts::OS, "macos" | "linux") {
        command
            .arg("-C")
            .arg(format!("link-arg=-Wl,-rpath,{}", native_dir.display()));
    }
}

fn sanitize_artifact_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
