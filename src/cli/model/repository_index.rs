use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::model::manifest::{IndexPackageVersion, Repository, RepositoryIndex, RepositoryType};
use crate::model::version::VersionConstraint;
use crate::sources::fetch::fetch_asset;
use crate::sources::git_source::{GitSource, install_git_source};

pub fn load_repository_index(repository: &Repository) -> Result<Option<RepositoryIndex>> {
    let Some((path, signature_path)) = repository_index_paths(repository)? else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read repository index {}", path.display()))?;
    if let Some(key) = &repository.key {
        let signature_path = signature_path.with_context(|| {
            format!(
                "repository {} has a key but no repository-index signature was found",
                repository.url
            )
        })?;
        let signature = std::fs::read_to_string(&signature_path)
            .with_context(|| format!("failed to read {}", signature_path.display()))?;
        verify_index_signature(&bytes, key, signature.trim()).with_context(|| {
            format!(
                "repository-index signature verification failed for {}",
                repository.url
            )
        })?;
    }

    let index = serde_json::from_slice::<RepositoryIndex>(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(index))
}

pub fn select_package_version(
    index: &RepositoryIndex,
    package: &str,
    constraint: Option<&str>,
) -> Result<Option<(String, IndexPackageVersion)>> {
    select_package_version_following_aliases(index, package, constraint, 0)
}

/// Resolve `package`, following `ref` aliases (`sdl` → `ref: "libsdl"`) within the
/// index, with a depth bound so a misconfigured cycle fails instead of looping.
fn select_package_version_following_aliases(
    index: &RepositoryIndex,
    name: &str,
    constraint: Option<&str>,
    depth: usize,
) -> Result<Option<(String, IndexPackageVersion)>> {
    if depth > 16 {
        bail!("repository index alias chain for {name:?} is too deep (possible cycle)");
    }
    let Some(package) = index.packages.get(name) else {
        return Ok(None);
    };
    if let Some(target) = &package.alias {
        return select_package_version_following_aliases(index, target, constraint, depth + 1);
    }
    let constraint = constraint
        .map(VersionConstraint::parse)
        .transpose()
        .with_context(|| format!("invalid dependency version constraint for {package:?}"))?;

    let mut candidates = Vec::new();
    for (version, entry) in &package.versions {
        let normalized = crate::model::validate::normalize_semver(version)?;
        if constraint
            .as_ref()
            .is_none_or(|constraint| constraint.matches(&normalized))
        {
            candidates.push((normalized, version.clone(), entry.clone()));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(candidates
        .pop()
        .map(|(_normalized, version, entry)| (version, entry)))
}

pub fn installed_version_satisfies(version: &str, constraint: &str) -> Result<bool> {
    let version = crate::model::validate::normalize_semver(version)?;
    Ok(VersionConstraint::parse(constraint)?.matches(&version))
}

pub fn repository_index_url(repository: &Repository) -> Option<String> {
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

pub fn local_repository_dir(repository: &Repository) -> Option<PathBuf> {
    if repository.kind != RepositoryType::File {
        return None;
    }
    match repository.url.strip_prefix("file://") {
        Some(path) => Some(PathBuf::from(path)),
        None => Some(PathBuf::from(&repository.url)),
    }
}

pub fn sign_index_file(index: &Path, secret_key: &str, output: Option<&Path>) -> Result<PathBuf> {
    let bytes =
        std::fs::read(index).with_context(|| format!("failed to read {}", index.display()))?;
    let secret = decode_prefixed_bytes(secret_key, 32)?;
    let signing_key = SigningKey::from_bytes(
        secret
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("ed25519 secret keys must be 32 bytes"))?,
    );
    let signature = signing_key.sign(&bytes);
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| index.with_extension("json.sig"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(
        &output,
        format!("ed25519:{}\n", BASE64.encode(signature.to_bytes())),
    )
    .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(output)
}

pub fn public_key_from_secret(secret_key: &str) -> Result<String> {
    let secret = decode_prefixed_bytes(secret_key, 32)?;
    let signing_key = SigningKey::from_bytes(
        secret
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("ed25519 secret keys must be 32 bytes"))?,
    );
    Ok(format!(
        "ed25519:{}",
        BASE64.encode(signing_key.verifying_key().to_bytes())
    ))
}

pub fn validate_public_key(public_key: &str) -> Result<()> {
    decode_prefixed_bytes(public_key, 32).map(|_| ())
}

fn repository_index_paths(repository: &Repository) -> Result<Option<(PathBuf, Option<PathBuf>)>> {
    if let Some(dir) = local_repository_dir(repository) {
        let index = dir.join("repository-index.json");
        if !index.is_file() {
            return Ok(None);
        }
        let signature = dir.join("repository-index.json.sig");
        return Ok(Some((index, signature.is_file().then_some(signature))));
    }

    if let Some(url) = repository_index_url(repository) {
        let index = fetch_asset(&url)?;
        let signature = fetch_asset(&format!("{url}.sig")).ok();
        return Ok(Some((index, signature)));
    }

    let source = GitSource::parse(&repository.url)?;
    let installed = install_git_source(&source)?;
    let base = if source.package_path.as_os_str().is_empty() {
        installed.destination.clone()
    } else {
        installed.destination.join(&source.package_path)
    };
    let index = base.join("repository-index.json");
    if !index.is_file() {
        return Ok(None);
    }
    let signature = base.join("repository-index.json.sig");
    Ok(Some((index, signature.is_file().then_some(signature))))
    // `installed` drops here, removing the temporary clone after the index is read.
}

fn verify_index_signature(index_bytes: &[u8], key: &str, signature: &str) -> Result<()> {
    let key = decode_prefixed_bytes(key, 32)?;
    let signature = decode_prefixed_bytes(signature, 64)?;
    let key = VerifyingKey::from_bytes(
        key.as_slice()
            .try_into()
            .map_err(|_| anyhow!("ed25519 public keys must be 32 bytes"))?,
    )?;
    let signature = Signature::from_slice(&signature)?;
    key.verify(index_bytes, &signature)?;
    Ok(())
}

fn decode_prefixed_bytes(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let raw = value.strip_prefix("ed25519:").unwrap_or(value).trim();
    if raw.len() == expected_len * 2 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return decode_hex(raw);
    }
    let decoded = BASE64
        .decode(raw)
        .with_context(|| "expected ed25519 material as base64 or hex")?;
    if decoded.len() != expected_len {
        bail!(
            "ed25519 material must be {expected_len} bytes, got {}",
            decoded.len()
        );
    }
    Ok(decoded)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        out.push(u8::from_str_radix(&value[index..index + 2], 16)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_verifies_index_bytes() {
        let secret = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let public = public_key_from_secret(secret).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("repository-index.json");
        std::fs::write(
            &index,
            b"{\"schema_version\":\"2026-07-01\",\"packages\":{}}\n",
        )
        .unwrap();

        let signature = sign_index_file(&index, secret, None).unwrap();
        let bytes = std::fs::read(&index).unwrap();
        let signature = std::fs::read_to_string(signature).unwrap();

        verify_index_signature(&bytes, &public, signature.trim()).unwrap();
        assert!(verify_index_signature(b"tampered", &public, signature.trim()).is_err());
    }

    #[test]
    fn selects_highest_matching_version() {
        let index: RepositoryIndex = serde_json::from_str(
            r#"{
              "schema_version": "2026-07-01",
              "packages": {
                "vendor/pkg": {
                  "versions": {
                    "1.0.0": {
                      "manifest": "vendor/pkg/pnlx.json",
                      "dist": {"url": "https://example.test/pkg", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                      "source": {"type": "git", "url": "https://example.test/pkg", "reference": "main"}
                    },
                    "1.2.0": {
                      "manifest": "vendor/pkg/pnlx.json",
                      "dist": {"url": "https://example.test/pkg", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                      "source": {"type": "git", "url": "https://example.test/pkg", "reference": "main"}
                    }
                  }
                }
              }
            }"#,
        )
        .unwrap();

        let (version, _) = select_package_version(&index, "vendor/pkg", Some(">=1.0.0 & <1.2.0"))
            .unwrap()
            .unwrap();
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn follows_package_alias_ref() {
        let index: RepositoryIndex = serde_json::from_str(
            r#"{
              "schema_version": "2026-07-01",
              "packages": {
                "sdl": {"ref": "libsdl"},
                "libsdl": {
                  "versions": {
                    "2.0.0": {
                      "manifest": "libsdl/pnlx.json",
                      "dist": {"url": "https://example.test/sdl", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                      "source": {"type": "git", "url": "https://example.test/sdl", "reference": "main"}
                    }
                  }
                }
              }
            }"#,
        )
        .unwrap();

        // `sdl` resolves through its `ref` to `libsdl`'s versions.
        let (version, entry) = select_package_version(&index, "sdl", None)
            .unwrap()
            .unwrap();
        assert_eq!(version, "2.0.0");
        assert_eq!(entry.manifest, "libsdl/pnlx.json");
    }

    #[test]
    fn detects_alias_cycle() {
        let index: RepositoryIndex = serde_json::from_str(
            r#"{
              "schema_version": "2026-07-01",
              "packages": {
                "a": {"ref": "b"},
                "b": {"ref": "a"}
              }
            }"#,
        )
        .unwrap();

        assert!(select_package_version(&index, "a", None).is_err());
    }
}
