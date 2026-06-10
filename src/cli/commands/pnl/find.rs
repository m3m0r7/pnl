//! `pnl find <glob>` — list packages available from the configured repositories
//! (plus the built-in default) that match an optional glob pattern.
//!
//! Each repository is enumerated cheaply when it publishes a
//! `repository-index.json` (fetched over HTTP for GitHub/https repositories, or
//! read from disk for local ones); otherwise pnl falls back to a shallow clone
//! and a bounded directory walk for `pnlx.json` files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::fetch::fetch_asset;
use crate::git_source::{GitSource, install_git_source};
use crate::glob::package_name_matches;
use crate::io::read_or_default;
use crate::manifest::{PnlManifest, Repository, RepositoryIndex, RepositoryType};

use super::install::resolved_repositories;

/// The minimal slice of a `pnlx.json` needed to list a package.
#[derive(Debug, Deserialize)]
struct PackageHead {
    name: String,
    version: String,
}

/// A package discovered in a repository, with the versions that repository offers.
struct FoundPackage {
    versions: Vec<String>,
    repository: String,
}

pub(super) fn find(root: &Path, pattern: Option<&str>) -> Result<()> {
    let manifest = read_or_default::<PnlManifest>(&root.join("pnl.json"))?;
    let repositories = resolved_repositories(&manifest);

    // The highest-priority repository that defines a package wins; lower-priority
    // repositories (the default sits last) never shadow an earlier hit.
    let mut found: BTreeMap<String, FoundPackage> = BTreeMap::new();
    for repository in &repositories {
        match enumerate_repository(repository) {
            Ok(packages) => {
                for (name, versions) in packages {
                    if pattern.is_some_and(|pattern| !package_name_matches(pattern, &name)) {
                        continue;
                    }
                    found.entry(name).or_insert(FoundPackage {
                        versions,
                        repository: repository.url.clone(),
                    });
                }
            }
            Err(error) => {
                crate::ui::warn(&format!("skipped repository {}: {error}", repository.url));
            }
        }
    }

    if found.is_empty() {
        match pattern {
            Some(pattern) => crate::ui::info(&format!("no packages match \"{pattern}\"")),
            None => crate::ui::info("no packages found in the configured repositories"),
        }
        return Ok(());
    }

    for (name, package) in &found {
        println!(
            "{} {} {}",
            name,
            package.versions.join(", "),
            crate::ui::dim(&package.repository),
        );
    }
    Ok(())
}

/// Enumerate every package a repository offers as `(name, versions)` pairs.
fn enumerate_repository(repository: &Repository) -> Result<Vec<(String, Vec<String>)>> {
    // Local repositories are read straight from disk.
    if let Some(dir) = local_repository_dir(repository) {
        if let Some(entries) = index_entries_at(&dir.join("repository-index.json")) {
            return Ok(entries);
        }
        return Ok(walk_packages(&dir));
    }

    // Remote repositories: try a published index first (one HTTP request), then
    // fall back to cloning and walking the tree.
    if let Some(url) = http_index_url(repository)
        && let Ok(path) = fetch_asset(&url)
        && let Some(entries) = index_entries_at(&path)
    {
        return Ok(entries);
    }

    enumerate_by_clone(repository)
}

/// A local on-disk directory for a `file` repository, if this is one.
fn local_repository_dir(repository: &Repository) -> Option<PathBuf> {
    if repository.kind != RepositoryType::File {
        return None;
    }
    match repository.url.strip_prefix("file://") {
        Some(path) => Some(PathBuf::from(path)),
        None => Some(PathBuf::from(&repository.url)),
    }
}

/// The raw HTTP URL of a repository's `repository-index.json`, when one can be
/// derived (GitHub tree/web URLs, or an explicit `https` repository base).
fn http_index_url(repository: &Repository) -> Option<String> {
    if repository.url.contains("github.com") {
        let source = GitSource::parse(&repository.url).ok()?;
        let branch = source.branch.clone().unwrap_or_else(|| "HEAD".to_owned());
        let package_path = source.package_path.to_string_lossy().replace('\\', "/");
        let prefix = if package_path.is_empty() {
            String::new()
        } else {
            format!("{package_path}/")
        };
        return Some(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}repository-index.json",
            source.vendor, source.name, branch, prefix,
        ));
    }
    if repository.kind == RepositoryType::Https {
        return Some(format!(
            "{}/repository-index.json",
            repository.url.trim_end_matches('/'),
        ));
    }
    None
}

/// Clone a repository shallowly and enumerate its packages from disk, preferring
/// an index file committed to the tree.
fn enumerate_by_clone(repository: &Repository) -> Result<Vec<(String, Vec<String>)>> {
    let source = GitSource::parse(&repository.url)?;
    let installed = install_git_source(&source)?;
    let base = if source.package_path.as_os_str().is_empty() {
        installed.destination.clone()
    } else {
        installed.destination.join(&source.package_path)
    };

    if let Some(entries) = index_entries_at(&base.join("repository-index.json")) {
        return Ok(entries);
    }
    Ok(walk_packages(&base))
    // `installed` drops here, removing the temporary clone.
}

/// Parse a `repository-index.json` at `path` into `(name, versions)` pairs.
/// Returns `None` when the file is absent or unparseable, so the caller can fall
/// back to a directory walk.
fn index_entries_at(path: &Path) -> Option<Vec<(String, Vec<String>)>> {
    let contents = std::fs::read_to_string(path).ok()?;
    let index: RepositoryIndex = serde_json::from_str(&contents).ok()?;
    Some(
        index
            .packages
            .into_iter()
            .map(|(name, package)| (name, package.versions.into_keys().collect()))
            .collect(),
    )
}

/// Walk a repository directory (bounded depth) collecting every package that has
/// a `pnlx.json`, supporting both `<packages>/<name>/` and
/// `<packages>/<vendor>/<name>/` layouts.
fn walk_packages(dir: &Path) -> Vec<(String, Vec<String>)> {
    let mut packages = Vec::new();
    collect_packages(dir, 3, &mut packages);
    packages
}

fn collect_packages(dir: &Path, depth: usize, out: &mut Vec<(String, Vec<String>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("pnlx.json");
        if manifest.is_file() {
            if let Some(head) = read_package_head(&manifest) {
                out.push((head.name, vec![head.version]));
            }
        } else if depth > 0 {
            collect_packages(&path, depth - 1, out);
        }
    }
}

/// Read just the name and version from a `pnlx.json`, leniently — `find` should
/// list a package even if its manifest carries fields pnl does not recognise.
fn read_package_head(path: &Path) -> Option<PackageHead> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}
