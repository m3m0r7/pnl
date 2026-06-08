use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::io::{read_json, write_json};
use crate::manifest::{PnlLock, PnlxManifest, PnlxPathmap};
use crate::platform::{GeneratedMetadata, current_platform};
use crate::validate::{ensure_platform_matches, validate_pnlx_pathmap_values};

pub(super) fn pnlx_workspace_dir(root: &Path) -> PathBuf {
    root.join("@pnlx")
}

pub(super) fn pnl_lock_path(root: &Path) -> PathBuf {
    pnlx_workspace_dir(root).join("pnlx-lock.json")
}

pub(super) fn pnlx_pathmap_path(root: &Path) -> PathBuf {
    pnlx_workspace_dir(root).join("pnlx-pathmap.json")
}

pub(super) fn install_extension_files(
    root: &Path,
    source: &Path,
    package: &str,
) -> Result<PathBuf> {
    let destination = installed_extension_dir(root, package);
    if destination.exists() {
        fs::remove_dir_all(&destination)
            .with_context(|| format!("failed to remove {}", destination.display()))?;
    }
    copy_package_directory(source, source, &destination)?;
    Ok(destination)
}

pub(super) fn installed_extension_dir(root: &Path, package: &str) -> PathBuf {
    let mut path = pnlx_workspace_dir(root).join("packages");
    for segment in package.split('/') {
        path.push(segment);
    }
    path
}

pub(super) fn write_pnlx_autoload(root: &Path) -> Result<()> {
    let autoload_path = pnlx_workspace_dir(root).join("autoload.php");
    let packages = installed_package_entrypoints(root)?;
    let metadata = GeneratedMetadata::current();
    let mut content = format!(
        r#"<?php

declare(strict_types=1);

/*
 * This file is generated. Manual edits may be overwritten.
 *
 * Generated at: {}
 * Generated on: {}
 * Generator OS: {}
 * PHP version: {}
 */

"#,
        metadata.generated_at, metadata.host, metadata.os, metadata.php_version
    );

    for entrypoint in packages {
        content.push_str("require_once __DIR__ . '/");
        content.push_str(&php_single_quoted_path(&entrypoint));
        content.push_str("';\n");
    }

    if let Some(parent) = autoload_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&autoload_path, content)
        .with_context(|| format!("failed to write {}", autoload_path.display()))?;
    Ok(())
}

pub(super) fn read_lock_for_current_platform(root: &Path) -> Result<PnlLock> {
    let platform = current_platform();
    let lock_path = pnl_lock_path(root);
    if !lock_path.exists() {
        return Ok(PnlLock::empty(platform));
    }

    let lock = read_json::<PnlLock>(&lock_path)?;
    if lock.platform == platform {
        Ok(lock)
    } else {
        Ok(PnlLock::empty(platform))
    }
}

pub(super) fn read_pathmap_for_current_platform(root: &Path) -> Result<PnlxPathmap> {
    let path = pnlx_pathmap_path(root);
    if !path.exists() {
        return Ok(PnlxPathmap::empty_current());
    }

    let pathmap = read_json::<PnlxPathmap>(&path)?;
    ensure_platform_matches(&pathmap.platform)?;
    validate_pnlx_pathmap_values(&pathmap)?;
    Ok(pathmap)
}

pub(super) fn write_pathmap(root: &Path, pathmap: &PnlxPathmap) -> Result<()> {
    write_json(&pnlx_pathmap_path(root), pathmap)
}

pub(super) fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

pub(super) fn relative_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn json_string(path: &Path) -> String {
    serde_json::to_string(&path.display().to_string()).expect("path string is serializable")
}

pub(super) fn absolutize(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub(super) fn file_url_for_path(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", path.display())
}

fn copy_package_directory(source_root: &Path, source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;

    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", source.display()))?;
        let source_path = entry.path();
        if should_skip_package_copy(source_root, &source_path) {
            continue;
        }

        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_package_directory(source_root, &source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn should_skip_package_copy(source_root: &Path, source_path: &Path) -> bool {
    let relative = source_path.strip_prefix(source_root).unwrap_or(source_path);
    relative == Path::new("src/generated")
        || relative.starts_with(".git")
        || relative.starts_with("@pnlx")
}

fn installed_package_entrypoints(root: &Path) -> Result<Vec<String>> {
    let packages_root = pnlx_workspace_dir(root).join("packages");
    if !packages_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut entrypoints = Vec::new();
    for vendor in fs::read_dir(&packages_root)
        .with_context(|| format!("failed to read {}", packages_root.display()))?
    {
        let vendor =
            vendor.with_context(|| format!("failed to read {}", packages_root.display()))?;
        if !vendor.path().is_dir() {
            continue;
        }

        for package in fs::read_dir(vendor.path())
            .with_context(|| format!("failed to read {}", vendor.path().display()))?
        {
            let package =
                package.with_context(|| format!("failed to read {}", vendor.path().display()))?;
            let package_root = package.path();
            let manifest_path = package_root.join("pnlx.json");
            if !manifest_path.is_file() {
                continue;
            }

            let manifest = read_json::<PnlxManifest>(&manifest_path)?;
            let entrypoint = package_root.join(&manifest.entrypoint);
            if entrypoint.is_file() {
                entrypoints.push(relative_to_pnlx(root, &entrypoint));
            }
        }
    }

    entrypoints.sort();
    Ok(entrypoints)
}

fn relative_to_pnlx(root: &Path, path: &Path) -> String {
    path.strip_prefix(pnlx_workspace_dir(root))
        .unwrap_or(path)
        .display()
        .to_string()
}

fn php_single_quoted_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('\'', "\\'")
}
