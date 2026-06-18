use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::interaction::Interaction;
use crate::manifest::PnlxManifest;
use crate::validate::validate_sha256;

#[derive(Debug, Serialize)]
struct InstallScriptMaterial {
    kind: String,
    platform: Option<String>,
    index: usize,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    contents_sha256: Option<String>,
}

/// Hash every executable install-script input in a stable representation.
///
/// `installation` commands are hashed as command strings, in execution order.
/// `self_build` is hashed as the package-relative script path plus the script's
/// current contents, so changing the script without changing `pnlx.json` is
/// still detected.
pub fn install_script_hash(package_root: &Path, manifest: &PnlxManifest) -> Result<Option<String>> {
    let mut materials = Vec::new();

    if let Some(script) = &manifest.self_build {
        let path = resolve_package_relative_path(package_root, script)?;
        let contents = std::fs::read(&path)
            .with_context(|| format!("failed to read self_build script {}", path.display()))?;
        materials.push(InstallScriptMaterial {
            kind: "self_build".to_owned(),
            platform: None,
            index: 0,
            value: script.clone(),
            contents_sha256: Some(sha256_hex(&contents)),
        });
    }

    for (platform, entry) in &manifest.installation {
        for (index, command) in entry.check_if_exists.iter().enumerate() {
            materials.push(InstallScriptMaterial {
                kind: "checkIfExists".to_owned(),
                platform: Some(platform.clone()),
                index,
                value: command.clone(),
                contents_sha256: None,
            });
        }
        for (index, command) in entry.install.iter().enumerate() {
            materials.push(InstallScriptMaterial {
                kind: "install".to_owned(),
                platform: Some(platform.clone()),
                index,
                value: command.clone(),
                contents_sha256: None,
            });
        }
    }

    if materials.is_empty() {
        return Ok(None);
    }

    let encoded = serde_json::to_vec(&materials).context("failed to encode install scripts")?;
    Ok(Some(sha256_hex(&encoded)))
}

pub fn verify_install_scripts(
    package_root: &Path,
    manifest: &PnlxManifest,
    interaction: &Interaction,
    allow_unverified: bool,
    allowed_hashes: &[String],
    trusted_source: bool,
) -> Result<Option<String>> {
    let Some(actual) = install_script_hash(package_root, manifest)? else {
        return Ok(None);
    };
    validate_allowed_hashes(allowed_hashes)?;

    if allowed_hashes
        .iter()
        .any(|hash| hash.eq_ignore_ascii_case(&actual))
    {
        crate::ui::warn(&format!(
            "install script hash {actual} was explicitly allowed"
        ));
        return Ok(Some(actual));
    }

    match manifest.install_script_hash.as_deref() {
        Some(expected) if expected.eq_ignore_ascii_case(&actual) => Ok(Some(actual)),
        Some(expected) => confirm_unverified(
            manifest,
            interaction,
            allow_unverified,
            trusted_source,
            &format!(
                "install script hash changed for {name}\n  expected: {expected}\n  actual:   {actual}",
                name = manifest.name
            ),
        )
        .map(|()| Some(actual)),
        None => confirm_unverified(
            manifest,
            interaction,
            allow_unverified,
            trusted_source,
            &format!(
                "{name} declares install scripts but does not carry install_script_hash {actual}",
                name = manifest.name
            ),
        )
        .map(|()| Some(actual)),
    }
}

fn confirm_unverified(
    manifest: &PnlxManifest,
    interaction: &Interaction,
    allow_unverified: bool,
    trusted_source: bool,
    message: &str,
) -> Result<()> {
    if allow_unverified {
        crate::ui::warn(message);
        crate::ui::warn("continuing because --allow-unverified-install-scripts was provided");
        return Ok(());
    }

    // Packages from a built-in authorized repository (see config.toml
    // `repositories.authorized`) are trusted to run their install scripts, so
    // they install without an interactive confirmation or warning.
    if trusted_source {
        return Ok(());
    }

    if interaction.assume_yes() {
        bail!(
            "{message}\nrefusing to continue under --yes; publish the package again or pass --allow-install-script-hash <sha256>"
        );
    }

    let proceed = interaction.confirm(
        &format!(
            "{message}\nInstall scripts can execute arbitrary commands. Continue installing {}?",
            manifest.name
        ),
        false,
    )?;
    if proceed {
        Ok(())
    } else {
        bail!("aborted because install scripts were not verified")
    }
}

pub fn resolve_package_relative_path(package_root: &Path, relative: &str) -> Result<PathBuf> {
    crate::validate::validate_relative_package_path("self_build", relative)?;
    let root = std::fs::canonicalize(package_root)
        .with_context(|| format!("failed to resolve package root {}", package_root.display()))?;
    let path = package_root.join(relative);
    let resolved = std::fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve package path {}", path.display()))?;
    if !resolved.starts_with(&root) {
        bail!(
            "self_build path {} resolves outside the package root",
            relative
        );
    }
    Ok(resolved)
}

pub fn validate_allowed_hashes(hashes: &[String]) -> Result<()> {
    for hash in hashes {
        validate_sha256(hash)?;
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{InstallationEntry, PnlxManifest};

    #[test]
    fn hashes_installation_commands_in_stable_order() {
        let mut manifest = PnlxManifest::default();
        manifest.installation.insert(
            "linux".to_owned(),
            InstallationEntry {
                install: vec!["apt-get install libfoo-dev".to_owned()],
                check_if_exists: vec!["pkg-config --exists foo".to_owned()],
            },
        );
        let dir = tempfile::tempdir().unwrap();

        let first = install_script_hash(dir.path(), &manifest).unwrap();
        let second = install_script_hash(dir.path(), &manifest).unwrap();
        assert_eq!(first, second);
        assert!(first.unwrap().chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn hashes_self_build_script_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.sh"), "echo one\n").unwrap();
        let mut manifest = PnlxManifest {
            self_build: Some("build.sh".to_owned()),
            ..PnlxManifest::default()
        };

        let first = install_script_hash(dir.path(), &manifest).unwrap();
        std::fs::write(dir.path().join("build.sh"), "echo two\n").unwrap();
        let second = install_script_hash(dir.path(), &manifest).unwrap();

        assert_ne!(first, second);
        manifest.self_build = None;
        assert_eq!(install_script_hash(dir.path(), &manifest).unwrap(), None);
    }

    #[test]
    fn rejects_self_build_traversal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_package_relative_path(dir.path(), "../build.sh").is_err());
    }
}
