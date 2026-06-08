use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::commands::pnlx::generate_installed_package_artifacts;
use crate::git_source::{GitSource, install_git_source};
use crate::io::{read_json, read_or_default, write_json};
use crate::manifest::{
    Dist, ExtensionRequirement, LockedExtension, LockedNativeLibrary, PnlManifest, PnlxManifest,
    RepositoryType, Source,
};
use crate::platform::now;
use crate::validate::{
    validate_pnl_manifest_values, validate_pnlx_manifest_values, validate_schema_version,
};

use super::bridge::compile_bridge_for_library;
use super::native::{
    generation_headers_from_resolved_header, resolve_header_for_native, resolve_native_library,
};
use super::package::{
    absolutize, file_url_for_path, install_extension_files, pnl_lock_path,
    read_lock_for_current_platform, read_pathmap_for_current_platform, sha256_file, sha256_hex,
    write_pathmap, write_pnlx_autoload,
};

pub(super) fn install(root: &Path, target: Option<&str>) -> Result<()> {
    let target = target.context("install target is required for the current implementation")?;
    let mut manifest = read_or_default::<PnlManifest>(&root.join("pnl.json"))?;
    validate_schema_version(&manifest.schema_version)?;
    validate_pnl_manifest_values(&manifest)?;

    match resolve_install_source(root, target)? {
        InstallSource::Local { path, source_url } => install_local_extension(
            root,
            &mut manifest,
            &path,
            ExtensionSource::File { source_url },
        ),
        InstallSource::Git(source) => install_git_extension(root, &mut manifest, target, source),
    }
}

#[derive(Debug, Clone)]
enum InstallSource {
    Local { path: PathBuf, source_url: String },
    Git(GitSource),
}

#[derive(Debug, Clone)]
enum ExtensionSource {
    File {
        source_url: String,
    },
    Git {
        source_url: String,
        reference: String,
        dist_url: String,
    },
}

fn install_git_extension(
    root: &Path,
    manifest: &mut PnlManifest,
    target: &str,
    source: GitSource,
) -> Result<()> {
    let installed = install_git_source(&source)?;
    let extension_root = installed.destination.join(&source.package_path);
    if !extension_root.join("pnlx.json").is_file() {
        bail!(
            "git source {} does not contain pnlx.json at the requested package path",
            source.url
        );
    }

    install_local_extension(
        root,
        manifest,
        &extension_root,
        ExtensionSource::Git {
            source_url: target.to_owned(),
            reference: installed.revision.clone(),
            dist_url: target.to_owned(),
        },
    )
}

fn resolve_install_source(root: &Path, target: &str) -> Result<InstallSource> {
    if target.starts_with("ftp://") || target.starts_with("ftps://") {
        bail!(
            "ftp install sources are not implemented yet; use a local path, file:// URL, or git URL"
        );
    }

    if let Some(path) = path_from_file_url(target) {
        let path = absolutize(root, &path);
        ensure_extension_source_path(&path, target)?;
        return Ok(InstallSource::Local {
            source_url: file_url_for_path(&path),
            path,
        });
    }

    let local_target = absolutize(root, Path::new(target));
    if local_target.join("pnlx.json").is_file() {
        return Ok(InstallSource::Local {
            source_url: file_url_for_path(&local_target),
            path: local_target,
        });
    }

    Ok(InstallSource::Git(GitSource::parse(target)?))
}

fn path_from_file_url(value: &str) -> Option<PathBuf> {
    let path = value.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost").unwrap_or(path);
    Some(PathBuf::from(path))
}

fn ensure_extension_source_path(path: &Path, original: &str) -> Result<()> {
    if path.join("pnlx.json").is_file() {
        Ok(())
    } else {
        bail!("{original} does not point to an extension root containing pnlx.json")
    }
}

fn install_local_extension(
    root: &Path,
    manifest: &mut PnlManifest,
    extension_root: &Path,
    source: ExtensionSource,
) -> Result<()> {
    let extension = read_json::<PnlxManifest>(&extension_root.join("pnlx.json"))?;
    validate_schema_version(&extension.schema_version)?;
    validate_pnlx_manifest_values(&extension)?;
    let installed_extension_root = install_extension_files(root, extension_root, &extension.name)?;

    manifest
        .extensions
        .entry(extension.name.clone())
        .or_insert_with(|| ExtensionRequirement {
            version: format!("={}", extension.version),
            required: true,
        });
    write_json(&root.join("pnl.json"), manifest)?;

    let mut lock = read_lock_for_current_platform(root)?;
    lock.generated_at = now();
    let mut locked_requires = BTreeMap::new();
    let mut pathmap = read_pathmap_for_current_platform(root)?;
    pathmap.generated_at = now();

    for (key, requirement) in &extension.requires {
        let native = resolve_native_library(root, manifest, key, &requirement.library_names)?;
        let header = resolve_header_for_native(
            extension_root,
            &native.path,
            key,
            &requirement.header_names,
        )?;
        let generation_headers =
            generation_headers_from_resolved_header(&header, &requirement.header_names);
        locked_requires.insert(
            key.clone(),
            LockedNativeLibrary {
                name: native.resolved_name.clone(),
                version: native.version.clone(),
                path: native.path.clone(),
                sha256: native.sha256.clone(),
            },
        );
        pathmap.requires.insert(key.clone(), native);
        pathmap.headers.insert(key.clone(), header);
        generate_installed_package_artifacts(
            &installed_extension_root,
            &extension,
            extension.name.rsplit('/').next().unwrap_or(key),
            key,
            &generation_headers,
        )?;
        if let Some(bridge) = compile_bridge_for_library(
            root,
            &installed_extension_root,
            key,
            &pathmap.requires[key],
        )? {
            pathmap.bridges.insert(key.clone(), bridge);
        }
    }

    let (source, dist) = source.lock_source(extension_root, &extension)?;
    lock.extensions.insert(
        extension.name.clone(),
        LockedExtension {
            version: extension.version.clone(),
            constraint: format!("={}", extension.version),
            source,
            dist,
            dependencies: BTreeMap::new(),
            requires: locked_requires,
        },
    );

    write_json(&pnl_lock_path(root), &lock)?;
    write_pathmap(root, &pathmap)?;
    write_pnlx_autoload(root)?;
    println!("installed extension {}", extension.name);
    Ok(())
}

impl ExtensionSource {
    fn lock_source(
        &self,
        extension_root: &Path,
        extension: &PnlxManifest,
    ) -> Result<(Source, Dist)> {
        match self {
            Self::File { source_url } => Ok((
                Source {
                    kind: RepositoryType::File,
                    url: source_url.clone(),
                    reference: extension.version.clone(),
                },
                Dist {
                    url: source_url.clone(),
                    sha256: sha256_file(&extension_root.join("pnlx.json"))?,
                },
            )),
            Self::Git {
                source_url,
                reference,
                dist_url,
            } => Ok((
                Source {
                    kind: RepositoryType::Git,
                    url: source_url.clone(),
                    reference: reference.clone(),
                },
                Dist {
                    url: dist_url.clone(),
                    sha256: sha256_hex(format!("{source_url}:{reference}").as_bytes()),
                },
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InstallSource, path_from_file_url, resolve_install_source};

    #[test]
    fn resolves_absolute_file_url_as_local_install_source() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("extension");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("pnlx.json"), "{}").unwrap();

        let source =
            resolve_install_source(temp.path(), &format!("file://{}", package.display())).unwrap();

        match source {
            InstallSource::Local { path, source_url } => {
                assert_eq!(path, package);
                assert!(source_url.starts_with("file://"));
                assert!(source_url.ends_with("/extension"));
            }
            InstallSource::Git(_) => panic!("expected local install source"),
        }
    }

    #[test]
    fn parses_localhost_file_url_path() {
        assert_eq!(
            path_from_file_url("file://localhost/tmp/pnl-package").unwrap(),
            std::path::PathBuf::from("/tmp/pnl-package")
        );
    }
}
