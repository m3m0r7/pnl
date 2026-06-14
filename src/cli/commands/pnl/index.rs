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

/// The minimal slice of a `pnlx.json` needed to index a package. A package can
/// either carry a `version` (a real package) or a `ref` (an alias `pnlx.json`,
/// e.g. `sdl/pnlx.json` with `"ref": "../libsdl"`), which is published as an
/// index alias rather than a version.
#[derive(Debug, Deserialize)]
struct PackageHead {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "ref", default)]
    alias: Option<String>,
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

    // An alias `pnlx.json` (`"ref": "../libsdl"`) publishes a redirect: the alias
    // name is the package directory, the target is the final path component of the
    // ref (a package name in this same index).
    if let Some(target) = &head.alias {
        let alias_name = head.name.clone().unwrap_or_else(|| {
            package_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let target_name = target
            .rsplit(['/', '\\'])
            .find(|segment| !segment.is_empty() && *segment != "..")
            .unwrap_or(target)
            .to_owned();
        index.packages.insert(
            alias_name,
            IndexPackage {
                alias: Some(target_name),
                versions: BTreeMap::new(),
            },
        );
        return Ok(());
    }

    let (Some(name), Some(version)) = (head.name.clone(), head.version.clone()) else {
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
    let reference = reference.unwrap_or(&version).to_owned();

    let entry = IndexPackageVersion {
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
        .entry(name)
        .or_insert_with(|| IndexPackage {
            alias: None,
            versions: BTreeMap::new(),
        })
        .versions
        .insert(version, entry);
    Ok(())
}

fn read_package_head(path: &Path) -> Option<PackageHead> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

#[cfg(test)]
mod tests {
    use super::generate_index;
    use crate::manifest::RepositoryIndex;

    #[test]
    fn publishes_pnlx_ref_as_index_alias() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // A real package …
        std::fs::create_dir(root.join("libsdl")).unwrap();
        std::fs::write(
            root.join("libsdl/pnlx.json"),
            r#"{"name": "libsdl", "version": "2.0.0"}"#,
        )
        .unwrap();

        // … and an alias package that redirects to it.
        std::fs::create_dir(root.join("sdl")).unwrap();
        std::fs::write(
            root.join("sdl/pnlx.json"),
            r#"{"schema_version": "2026-07-01", "ref": "../libsdl"}"#,
        )
        .unwrap();

        let output = root.join("repository-index.json");
        generate_index(root, "https://example.test/packages", Some(&output), None).unwrap();

        let index: RepositoryIndex =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();

        let alias = index.packages.get("sdl").expect("alias entry present");
        assert_eq!(alias.alias.as_deref(), Some("libsdl"));
        assert!(alias.versions.is_empty());

        let real = index.packages.get("libsdl").expect("real entry present");
        assert!(real.alias.is_none());
        assert!(real.versions.contains_key("2.0.0"));
    }
}
