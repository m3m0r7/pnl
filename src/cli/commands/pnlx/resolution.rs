use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::io::read_json;
use crate::manifest::{PnlxHeader, PnlxManifest, PnlxPathmap, ResolvedHeader};

pub(super) fn resolve_headers_from_pathmap(
    root: &Path,
    library_key: &str,
    manifest: &PnlxManifest,
) -> Result<Option<Vec<PathBuf>>> {
    let Some(project_root) = find_project_root(root) else {
        return Ok(None);
    };
    let pathmap = read_json::<PnlxPathmap>(
        &crate::workspace::workspace_dir(&project_root).join(crate::config::PATHMAP_FILE),
    )?;
    let Some(header) = pathmap.headers.get(library_key) else {
        return Ok(None);
    };

    let primary = resolve_pathmap_header(&project_root, header);
    let mut headers = vec![primary.clone()];
    if let Some(requirement) = manifest.requires.get(library_key)
        && let Some(include_root) =
            include_root_from_resolved_header(&primary, &requirement.header_names)
    {
        for name in &requirement.header_names {
            let candidate = include_root.join(name);
            if candidate.is_file() {
                headers.push(candidate);
            }
        }
    }

    headers.sort();
    headers.dedup();
    Ok(Some(headers))
}

pub(super) fn resolve_headers<'a>(
    target: &str,
    manifest: &'a PnlxManifest,
) -> Result<Vec<&'a PnlxHeader>> {
    if manifest.headers.is_empty() {
        return Ok(Vec::new());
    }

    if manifest.headers.len() == 1 {
        return Ok(vec![&manifest.headers[0]]);
    }

    let headers = manifest
        .headers
        .iter()
        .filter(|header| header_matches_target(header, target))
        .collect::<Vec<_>>();
    if headers.is_empty() {
        bail!("could not resolve header for {target}; add an unambiguous pnlx.json headers entry");
    }

    Ok(headers)
}

pub(super) fn resolve_library_key(
    target: &str,
    package_leaf: &str,
    manifest: &PnlxManifest,
) -> Result<String> {
    if manifest.requires.len() == 1 {
        return Ok(manifest
            .requires
            .keys()
            .next()
            .expect("len checked")
            .to_owned());
    }

    for candidate in [target, package_leaf] {
        if manifest.requires.contains_key(candidate) {
            return Ok(candidate.to_owned());
        }
    }

    bail!("could not resolve native library key for {target}; use --library-key");
}

pub(super) fn symbol_prefix_for_library(
    manifest: &PnlxManifest,
    library_key: &str,
) -> Option<String> {
    manifest
        .requires
        .get(library_key)
        .and_then(|requirement| requirement.symbol_prefix.clone())
}

pub(super) fn split_class(class: &str) -> Result<(String, String)> {
    let normalized = class.replace("\\\\", "\\");
    let (namespace, class_name) = normalized
        .rsplit_once('\\')
        .with_context(|| format!("class must include a namespace: {class}"))?;
    Ok((namespace.to_owned(), class_name.to_owned()))
}

pub(super) fn sanitize_artifact_stem(value: &str) -> String {
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

pub(super) fn symbol_prefix_from_library_key(value: &str) -> String {
    let mut prefix = value;
    if let Some((head, tail)) = value.rsplit_once('-')
        && tail.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
    {
        prefix = head;
    }

    prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned()
}

fn find_project_root(root: &Path) -> Option<PathBuf> {
    let mut current = std::fs::canonicalize(root).ok()?;
    loop {
        if crate::workspace::workspace_dir(&current)
            .join(crate::config::PATHMAP_FILE)
            .is_file()
        {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn resolve_pathmap_header(project_root: &Path, header: &ResolvedHeader) -> PathBuf {
    let path = PathBuf::from(&header.path);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
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

fn header_matches_target(header: &PnlxHeader, target: &str) -> bool {
    let target = target.to_ascii_lowercase();
    header.name.to_ascii_lowercase().contains(&target)
        || header.path.to_ascii_lowercase().contains(&target)
}
