//! `pnl repo index <packages-dir>` — generate a `repository-index.json` for a
//! directory of packages so a repository (e.g. the default `pnl-packages`) can
//! publish a catalogue that `pnl find` reads without cloning.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

use crate::io::write_json;
use crate::manifest::{
    Dist, IndexPackage, IndexPackageVersion, RepositoryIndex, RepositoryType, Source,
};

use super::package::tree_sha256;

/// The minimal slice of a `pnlx.json` needed to index a package.
#[derive(Debug, Deserialize)]
struct PackageHead {
    name: String,
    version: String,
}

/// Walk `packages_dir`, and write a `repository-index.json` listing every
/// package found. `base_url` is the installable base each package is appended to
/// (e.g. `https://github.com/m3m0r7/pnl-packages/tree/main/packages`);
/// `reference` overrides the git reference recorded per version (defaults to the
/// package version).
pub(super) fn generate_index(
    packages_dir: &Path,
    base_url: &str,
    output: Option<&Path>,
    reference: Option<&str>,
) -> Result<()> {
    let mut index = RepositoryIndex::empty();
    collect(
        packages_dir,
        packages_dir,
        base_url,
        reference,
        3,
        &mut index,
    )?;

    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| packages_dir.join("repository-index.json"));
    write_json(&output, &index)?;
    crate::ui::success(&format!(
        "indexed {} package(s) into {}",
        index.packages.len(),
        output.display()
    ));
    Ok(())
}

fn collect(
    root: &Path,
    dir: &Path,
    base_url: &str,
    reference: Option<&str>,
    depth: usize,
    index: &mut RepositoryIndex,
) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("pnlx.json");
        if manifest_path.is_file() {
            index_package(root, &path, &manifest_path, base_url, reference, index)?;
        } else if depth > 0 {
            collect(root, &path, base_url, reference, depth - 1, index)?;
        }
    }
    Ok(())
}

fn index_package(
    root: &Path,
    package_dir: &Path,
    manifest_path: &Path,
    base_url: &str,
    reference: Option<&str>,
    index: &mut RepositoryIndex,
) -> Result<()> {
    let Some(head) = read_package_head(manifest_path) else {
        return Ok(());
    };

    // The package's path relative to the repository root, in forward slashes —
    // used both as the manifest pointer and to build the installable URL.
    let relative_dir = package_dir
        .strip_prefix(root)
        .unwrap_or(package_dir)
        .to_string_lossy()
        .replace('\\', "/");
    let url = format!("{}/{relative_dir}", base_url.trim_end_matches('/'));
    let sha256 = tree_sha256(package_dir)?;
    let reference = reference.unwrap_or(&head.version).to_owned();

    let version = IndexPackageVersion {
        manifest: format!("{relative_dir}/pnlx.json"),
        dist: Dist {
            url: url.clone(),
            sha256,
        },
        source: Source {
            kind: RepositoryType::Git,
            url,
            reference,
        },
    };

    index
        .packages
        .entry(head.name)
        .or_insert_with(|| IndexPackage {
            versions: BTreeMap::new(),
        })
        .versions
        .insert(head.version, version);
    Ok(())
}

fn read_package_head(path: &Path) -> Option<PackageHead> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}
