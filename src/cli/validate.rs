use std::path::Path;

use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, Utc};

use crate::SCHEMA_VERSION;
use crate::io::read_json;
use crate::manifest::{Platform, PnlLock, PnlManifest, PnlxManifest, PnlxPathmap};
use crate::platform::current_platform;

pub fn validate_pnl_workspace(root: &Path) -> Result<()> {
    let manifest = read_json::<PnlManifest>(&root.join(crate::config::PNL_MANIFEST_FILE))?;
    validate_schema_version(&manifest.schema_version)?;
    validate_pnl_manifest_values(&manifest)?;

    let workspace = crate::workspace::workspace_dir(root);
    let lock_path = root.join(crate::config::LOCK_FILE);
    let lock = if lock_path.exists() {
        let lock = read_json::<PnlLock>(&lock_path)?;
        validate_schema_version(&lock.schema_version)?;
        ensure_platform_matches(&lock.platform)?;
        validate_pnl_lock_values(&lock)?;
        Some(lock)
    } else {
        None
    };

    let pathmap_path = workspace.join(crate::config::PATHMAP_FILE);
    let pathmap = if pathmap_path.exists() {
        let pathmap = read_json::<PnlxPathmap>(&pathmap_path)?;
        validate_schema_version(&pathmap.schema_version)?;
        ensure_platform_matches(&pathmap.platform)?;
        validate_pnlx_pathmap_values(&pathmap)?;
        Some(pathmap)
    } else {
        None
    };

    validate_workspace_consistency(&manifest, lock.as_ref(), pathmap.as_ref())?;

    crate::ui::success("pnl workspace is valid");
    Ok(())
}

fn validate_workspace_consistency(
    manifest: &PnlManifest,
    lock: Option<&PnlLock>,
    pathmap: Option<&PnlxPathmap>,
) -> Result<()> {
    let Some(lock) = lock else {
        if manifest
            .extensions
            .values()
            .any(|requirement| requirement.required)
        {
            bail!("pnl.json declares required extensions but pnlx-lock.json is missing");
        }
        return Ok(());
    };

    for (name, requirement) in &manifest.extensions {
        if requirement.required && !lock.extensions.contains_key(name) {
            bail!("pnl.json requires {name}, but it is missing from pnlx-lock.json");
        }
    }

    let Some(pathmap) = pathmap else {
        if lock
            .extensions
            .values()
            .any(|extension| !extension.requires.is_empty())
        {
            bail!("pnlx-lock.json records native requirements but pnlx-pathmap.json is missing");
        }
        return Ok(());
    };

    for (extension_name, extension) in &lock.extensions {
        for native in extension.requires.keys() {
            if !pathmap.requires.contains_key(native) {
                bail!(
                    "{extension_name} requires {native}, but it is missing from pnlx-pathmap.json requires"
                );
            }
            if !pathmap.headers.contains_key(native) {
                bail!(
                    "{extension_name} requires {native}, but it is missing from pnlx-pathmap.json headers"
                );
            }
        }
    }

    Ok(())
}

pub fn validate_pnlx_workspace(root: &Path) -> Result<()> {
    let manifest = read_json::<PnlxManifest>(&root.join(crate::config::PNLX_MANIFEST_FILE))?;
    validate_schema_version(&manifest.schema_version)?;
    validate_pnlx_manifest_values(&manifest)?;
    for example in &manifest.examples {
        if !root.join(example).is_file() {
            bail!("examples entry {example} does not exist in the package");
        }
    }

    crate::ui::success("pnlx workspace is valid");
    Ok(())
}

pub fn validate_pnl_manifest_values(manifest: &PnlManifest) -> Result<()> {
    validate_relative_package_path("output_dir", &manifest.output_dir)?;
    for repository in &manifest.repositories {
        if let Some(key) = &repository.key {
            crate::repository_index::validate_public_key(key)?;
        }
    }
    for (name, requirement) in &manifest.extensions {
        validate_package_name(name)?;
        validate_version_constraint(&requirement.version)?;
    }

    Ok(())
}

pub fn validate_pnl_lock_values(lock: &PnlLock) -> Result<()> {
    validate_rfc3339_datetime("generated_at", &lock.generated_at)?;
    for (name, extension) in &lock.extensions {
        validate_package_name(name)?;
        validate_semver(&extension.version)?;
        validate_version_constraint(&extension.constraint)?;
        for (dependency, constraint) in &extension.dependencies {
            validate_package_name(dependency)?;
            validate_version_constraint(constraint)?;
        }
        for native in extension.requires.values() {
            validate_semver(&native.version)?;
        }
    }

    Ok(())
}

pub fn validate_pnlx_manifest_values(manifest: &PnlxManifest) -> Result<()> {
    validate_package_name(&manifest.name)?;
    validate_semver(&manifest.version)?;
    validate_relative_package_path("entrypoint", &manifest.entrypoint)?;
    if let Some(hash) = &manifest.install_script_hash {
        validate_sha256(hash)?;
    }
    if let Some(self_build) = &manifest.self_build {
        validate_relative_package_path("self_build", self_build)?;
        if !manifest.installation.is_empty() {
            bail!("pnlx.json self_build cannot be used together with installation commands");
        }
    }
    if manifest.requires.is_empty() {
        bail!("pnlx.json requires must contain at least one native library requirement");
    }
    for requirement in manifest.requires.values() {
        for header in &requirement.header_names {
            validate_relative_package_path("header_names", header)?;
        }
        for library in &requirement.library_names {
            validate_library_name(library.name())?;
        }
    }
    for requirement in manifest.requires.values() {
        validate_version_constraint(&requirement.version)?;
    }
    for entries in manifest.dependencies.values() {
        for entry in entries {
            for library in &entry.library_names {
                validate_library_name(library.name())?;
            }
            // `package_names` may be bare names, `file://`, `git@`, or paths — they
            // are resolved like an install target, so they are not constrained here.
        }
    }
    for example in &manifest.examples {
        validate_relative_package_path("examples", example)?;
    }

    Ok(())
}

/// Package-relative file references (e.g. `examples` entries) must stay inside
/// the package directory.
pub fn validate_relative_package_path(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{field} entries must not be empty");
    }
    if value.contains('\\') {
        bail!("{field} entries must use / as the path separator: {value}");
    }
    let path = std::path::Path::new(value);
    if path.is_absolute() {
        bail!("{field} entries must be paths relative to the package root: {value}");
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("{field} entries must not contain ..: {value}");
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::RootDir | std::path::Component::Prefix(_)
        )
    }) {
        bail!("{field} entries must be package-relative paths: {value}");
    }
    Ok(())
}

pub fn validate_sha256(value: &str) -> Result<()> {
    if value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(())
    } else {
        bail!("sha256 values must be 64 hexadecimal characters: {value}")
    }
}

fn validate_library_name(value: &str) -> Result<()> {
    if value.contains('/') || value.contains('\\') || value.contains("..") {
        bail!("library_names entries must be plain file names: {value}");
    }
    Ok(())
}

pub fn validate_pnlx_pathmap_values(pathmap: &PnlxPathmap) -> Result<()> {
    validate_rfc3339_datetime("generated_at", &pathmap.generated_at)?;
    for native in pathmap.requires.values() {
        validate_semver(&native.version)?;
    }

    Ok(())
}

pub fn validate_schema_version(version: &str) -> Result<()> {
    if NaiveDate::parse_from_str(version, "%Y-%m-%d").is_err() {
        bail!("schema_version must be a valid YYYY-MM-DD date: {version}");
    }

    if version != SCHEMA_VERSION {
        bail!("unsupported schema_version {version}; expected {SCHEMA_VERSION}");
    }

    Ok(())
}

pub fn validate_rfc3339_datetime(field: &str, value: &str) -> Result<()> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("{field} must be an RFC3339 date-time: {value}"))
}

pub fn validate_semver(version: &str) -> Result<()> {
    normalize_semver(version).map(|_| ())
}

/// Parse a version string, tolerating the partial versions real C libraries use
/// (e.g. `1.5`, `74.2`) and bare numeric versions (e.g. argon2's `20190702`) by
/// padding the missing components with zero.
pub fn normalize_semver(version: &str) -> Result<semver::Version> {
    let (core, suffix) = match version.find(['-', '+']) {
        Some(index) => (&version[..index], &version[index..]),
        None => (version, ""),
    };

    let mut parts = core.split('.').collect::<Vec<_>>();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
    {
        bail!("invalid semantic version: {version}");
    }
    while parts.len() < 3 {
        parts.push("0");
    }

    let normalized = format!("{}{suffix}", parts[..3].join("."));
    semver::Version::parse(&normalized)
        .map_err(|err| anyhow::anyhow!("invalid semantic version {version}: {err}"))
}

pub use crate::version::validate_version_constraint;

pub fn ensure_platform_matches(platform: &Platform) -> Result<()> {
    let current = current_platform();
    if *platform == current {
        Ok(())
    } else {
        bail!(
            "platform mismatch: file is {:?}, current environment is {:?}",
            platform,
            current
        );
    }
}

pub fn validate_package_name(name: &str) -> Result<()> {
    let mut parts = name.split('/');
    let vendor = parts.next().unwrap_or_default();
    let package = parts.next().unwrap_or_default();
    if parts.next().is_some() || vendor.is_empty() || package.is_empty() {
        bail!("package name must be vendor/extension: {name}");
    }
    sanitize_package_segment(vendor)?;
    sanitize_package_segment(package)?;
    Ok(())
}

pub fn sanitize_package_segment(value: &str) -> Result<String> {
    let normalized = value.to_ascii_lowercase();
    if normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
        && normalized
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
    {
        Ok(normalized)
    } else {
        bail!("invalid package segment: {value}");
    }
}
