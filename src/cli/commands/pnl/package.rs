use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::io::{read_json, write_json};
use crate::manifest::{PnlLock, PnlxManifest, PnlxPathmap};
use crate::platform::current_platform;
use crate::validate::{ensure_platform_matches, validate_pnlx_pathmap_values};

pub(super) fn pnlx_workspace_dir(root: &Path) -> PathBuf {
    crate::workspace::workspace_dir(root)
}

pub(super) fn pnl_lock_path(root: &Path) -> PathBuf {
    // The lockfile lives next to pnl.json (not inside the configurable output
    // dir) so it has a fixed, committable location independent of `output_dir`.
    root.join(crate::config::LOCK_FILE)
}

pub(super) fn pnlx_pathmap_path(root: &Path) -> PathBuf {
    pnlx_workspace_dir(root).join(crate::config::PATHMAP_FILE)
}

pub(super) fn install_extension_files(
    packages_root: &Path,
    source: &Path,
    package: &str,
    version: &str,
) -> Result<PathBuf> {
    let destination = extension_install_dir(packages_root, package, version);
    let parent = destination
        .parent()
        .with_context(|| format!("{} has no parent directory", destination.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let temp = parent.join(format!(".{version}.{}.tmp", std::process::id()));
    let backup = parent.join(format!(".{version}.{}.bak", std::process::id()));
    if temp.exists() {
        fs::remove_dir_all(&temp)
            .with_context(|| format!("failed to remove {}", temp.display()))?;
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to remove {}", backup.display()))?;
    }

    copy_package_directory(source, source, &temp)?;

    let had_previous = destination.exists();
    if had_previous {
        fs::rename(&destination, &backup).with_context(|| {
            format!(
                "failed to move {} to {}",
                destination.display(),
                backup.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&temp, &destination) {
        if had_previous {
            let _ = fs::rename(&backup, &destination);
        }
        bail!(
            "failed to install {} to {}: {error}",
            source.display(),
            destination.display()
        );
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to remove {}", backup.display()))?;
    }
    Ok(destination)
}

/// The directory holding every installed version of a package:
/// `<workspace>/packages/<vendor>/<package>`.
pub(super) fn installed_package_dir(root: &Path, package: &str) -> PathBuf {
    package_dir_in(&pnlx_workspace_dir(root).join("packages"), package)
}

/// The directory holding every installed version of a package inside a given
/// `packages/` root (the workspace's, or a parent package's subtree for a
/// dependency installed nested under it).
pub(super) fn package_dir_in(packages_root: &Path, package: &str) -> PathBuf {
    let mut path = packages_root.to_path_buf();
    for segment in package.split('/') {
        path.push(segment);
    }
    path
}

/// A specific installed version of a package inside a given `packages/` root.
pub(super) fn extension_install_dir(packages_root: &Path, package: &str, version: &str) -> PathBuf {
    package_dir_in(packages_root, package).join(version)
}

pub(super) fn write_pnlx_autoload(root: &Path) -> Result<()> {
    let workspace = pnlx_workspace_dir(root);
    let packages = collect_installed_packages(root)?
        .into_iter()
        .map(|package| php_single_quoted_path(&package.entrypoint))
        .collect::<Vec<_>>();
    let composites = collect_composite_requires(root, &workspace)?;

    // The absolute pnl.json path as it exists right now (install/init time),
    // baked into autoload.php so the runtime locates it without walking cwd.
    let manifest_path = root.join(crate::config::PNL_MANIFEST_FILE);
    let manifest_path = fs::canonicalize(&manifest_path)
        .unwrap_or(manifest_path)
        .to_string_lossy()
        .into_owned();
    crate::generate::generate_autoload_php(
        &workspace.join(crate::config::AUTOLOAD_FILE),
        env!("CARGO_PKG_VERSION"),
        &packages,
        &composites,
        &manifest_path,
    )?;
    crate::generate::generate_ide_helper_php(&workspace.join("ide-helper.php"))?;
    // Keep the self-contained SDK runtime in sync with the generating binary.
    write_runtime_assets(root)?;
    Ok(())
}

/// Composite class files (see `pnl compose`) recorded in `pnl.json`, returned as
/// autoload require paths relative to the workspace — only those whose generated
/// file actually exists on disk.
fn collect_composite_requires(root: &Path, workspace: &Path) -> Result<Vec<String>> {
    let manifest_path = root.join(crate::config::PNL_MANIFEST_FILE);
    if !manifest_path.is_file() {
        return Ok(Vec::new());
    }
    let manifest = read_json::<crate::manifest::PnlManifest>(&manifest_path)?;
    let mut requires = Vec::new();
    for fqn in manifest.composites.keys() {
        let class = fqn.rsplit('\\').next().unwrap_or(fqn);
        // The class file, then its optional global-functions file (sorted so
        // `<Class>.php` is required before `<Class>Functions.php`, which delegates
        // to the class).
        for relative in [
            format!("composites/{class}.php"),
            format!("composites/{class}Functions.php"),
        ] {
            if workspace.join(&relative).is_file() {
                requires.push(php_single_quoted_path(&relative));
            }
        }
    }
    requires.sort();
    Ok(requires)
}

/// Copy the SDK runtime tree and support library into `@pnlx/runtime/`.
fn write_runtime_assets(root: &Path) -> Result<()> {
    let runtime_dir = pnlx_workspace_dir(root).join("runtime");
    let marker = runtime_dir.join(".pnl-version");
    let version = env!("CARGO_PKG_VERSION");

    write_sdk_runtime(root)?;
    write_support_library(root)?;

    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("failed to create {}", runtime_dir.display()))?;
    fs::write(&marker, version).with_context(|| format!("failed to write {}", marker.display()))?;
    Ok(())
}

/// Expand the compiled support library (schema-validation FFI) into
/// `@pnlx/runtime/<lib>` so the PHP runtime can load it. Prefers the bytes
/// embedded in this binary (release builds); falls back to the cdylib sitting
/// next to the running executable (development builds). A no-op when neither is
/// available — the runtime then simply skips re-validation.
fn write_support_library(root: &Path) -> Result<()> {
    let lib_name = format!(
        "{}pnl{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );

    let bytes = if !crate::SUPPORT_LIB.is_empty() {
        Some(crate::SUPPORT_LIB.to_vec())
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(&lib_name)))
            .filter(|path| path.is_file())
            .and_then(|path| fs::read(&path).ok())
    };

    let Some(bytes) = bytes else {
        return Ok(());
    };

    let dest = pnlx_workspace_dir(root).join("runtime").join(&lib_name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&dest, &bytes).with_context(|| format!("failed to write {}", dest.display()))?;
    Ok(())
}

/// Write the embedded, self-contained copy of the PHP SDK runtime into
/// `@pnlx/runtime/`, so `@pnlx/autoload.php`'s fallback autoloader can resolve
/// `Pnlx\*` without an external autoloader.
fn write_sdk_runtime(root: &Path) -> Result<()> {
    write_embedded_dir(
        &crate::sdk_assets::SDK_DIR,
        &pnlx_workspace_dir(root).join("runtime"),
    )
}

/// Recursively write an embedded directory tree under `dest_root`, mirroring the
/// paths (each file's path is relative to the embedded root).
fn write_embedded_dir(dir: &include_dir::Dir<'_>, dest_root: &Path) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(sub) => write_embedded_dir(sub, dest_root)?,
            include_dir::DirEntry::File(file) => {
                let dest = dest_root.join(file.path());
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                fs::write(&dest, file.contents())
                    .with_context(|| format!("failed to write {}", dest.display()))?;
            }
        }
    }
    Ok(())
}

/// The path of the root `pnlx-lock.json` relative to the generated workspace
/// directory (where `autoload.php` lives), e.g. `../pnlx-lock.json` for the
/// default `@pnlx` output dir.
fn lock_path_relative_to_workspace(root: &Path) -> String {
    let depth = Path::new(&crate::workspace::output_dir_name(root))
        .components()
        .count()
        .max(1);
    format!("{}pnlx-lock.json", "../".repeat(depth))
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
    // Record where the lock is (relative to this pathmap's directory) so the
    // generated autoload can locate it without assuming a fixed `../` layout.
    let mut pathmap = pathmap.clone();
    pathmap.lock = lock_path_relative_to_workspace(root);
    // The absolute pnl.json path verified at install/init time (a record for
    // tooling; Runtime resolution uses the autoload's PNLX_PROJECT_MANIFEST).
    let manifest_path = root.join(crate::config::PNL_MANIFEST_FILE);
    pathmap.manifest = fs::canonicalize(&manifest_path)
        .unwrap_or(manifest_path)
        .to_string_lossy()
        .into_owned();
    write_json(&pnlx_pathmap_path(root), &pathmap)
}

pub(super) fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

/// Content digest of a package directory used as an integrity signature.
///
/// Files are sorted by relative path (A–z); each file's content is hashed with
/// sha256, then all of those per-file hashes are hashed together into a single
/// sha256. Generated output, `.git`, and the workspace directory are excluded so
/// the digest reflects only the package's own source.
pub(super) fn tree_sha256(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_package_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for relative in &files {
        let file_hash = sha256_file(&root.join(relative))?;
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_package_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let path = entry
            .with_context(|| format!("failed to read {}", dir.display()))?
            .path();
        if should_skip_package_copy(root, &path) {
            continue;
        }
        if path.is_dir() {
            collect_package_files(root, &path, files)?;
        } else {
            files.push(relative_slash_path(root, &path));
        }
    }
    Ok(())
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
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
    url::Url::from_file_path(&path)
        .map(String::from)
        .unwrap_or_else(|()| format!("file://{}", path.display()))
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
    relative == Path::new(crate::config::GENERATED_DIR)
        || relative.starts_with(".git")
        || relative.starts_with(crate::workspace::output_dir_name(source_root).as_str())
}

/// One installed package version, as needed to build `@pnlx/autoload.php`.
struct InstalledPackage {
    /// Generated entrypoint, relative to the `@pnlx` workspace directory.
    entrypoint: String,
}

fn collect_installed_packages(root: &Path) -> Result<Vec<InstalledPackage>> {
    let mut packages = Vec::new();
    collect_packages_in(
        root,
        &pnlx_workspace_dir(root).join("packages"),
        &mut packages,
    )?;
    packages.sort_by(|a, b| a.entrypoint.cmp(&b.entrypoint));
    Ok(packages)
}

/// Walk a `packages/` directory, collecting every installed package's entrypoint
/// and recursing into each version's own nested `packages/` (a package's private
/// dependencies installed under it), so the autoloader covers nested dependencies.
fn collect_packages_in(
    root: &Path,
    packages_root: &Path,
    packages: &mut Vec<InstalledPackage>,
) -> Result<()> {
    if !packages_root.is_dir() {
        return Ok(());
    }
    for vendor in fs::read_dir(packages_root)
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
            if !package.path().is_dir() {
                continue;
            }
            // Each package holds one directory per installed version.
            for version in fs::read_dir(package.path())
                .with_context(|| format!("failed to read {}", package.path().display()))?
            {
                let version = version
                    .with_context(|| format!("failed to read {}", package.path().display()))?;
                let manifest_path = version.path().join(crate::config::PNLX_MANIFEST_FILE);
                if manifest_path.is_file() {
                    let manifest = read_json::<PnlxManifest>(&manifest_path)?;
                    let entrypoint = version.path().join(&manifest.entrypoint);
                    if entrypoint.is_file() {
                        packages.push(InstalledPackage {
                            entrypoint: relative_to_pnlx(root, &entrypoint),
                        });
                    }
                }
                // Recurse into this version's nested dependency packages.
                collect_packages_in(root, &version.path().join("packages"), packages)?;
            }
        }
    }
    Ok(())
}

/// The fully qualified generated entity class name (namespace + `class_prefix` +
/// base class), or `None` when the manifest class carries no namespace.
pub(super) fn entity_class_fqn(manifest: &PnlxManifest) -> Option<String> {
    let normalized = manifest.class.replace("\\\\", "\\");
    let (namespace, class_name) = normalized.rsplit_once('\\')?;
    Some(format!(
        "{namespace}\\{}{class_name}",
        manifest.class_prefix
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(version_dir: &Path) {
        std::fs::create_dir_all(version_dir).unwrap();
        let manifest = crate::manifest::PnlxManifest {
            entrypoint: "index.php".to_owned(),
            ..crate::manifest::PnlxManifest::default()
        };
        crate::io::write_json(
            &version_dir.join(crate::config::PNLX_MANIFEST_FILE),
            &manifest,
        )
        .unwrap();
        std::fs::write(version_dir.join("index.php"), "<?php\n").unwrap();
    }

    #[test]
    fn collect_installed_packages_recurses_into_nested_dependencies() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        let packages = pnlx_workspace_dir(root).join("packages");

        // A top-level package and a dependency nested under its own subtree.
        write_package(&packages.join("acme/parent/1.0.0"));
        write_package(&packages.join("acme/parent/1.0.0/packages/acme/child/2.0.0"));

        let found = collect_installed_packages(root).unwrap();
        assert_eq!(
            found.len(),
            2,
            "expected the parent and its nested child, found {}",
            found.len()
        );
    }
}
