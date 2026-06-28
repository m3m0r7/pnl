//! Built-in configuration, baked in from `config.toml` at build time and (for
//! the repository endpoints) overridable per project via the `config` block in
//! `pnl.json`.
//!
//! The hardcoded constants used to live scattered across the crate; they are now
//! a single source of truth in `config.toml`, which `build.rs` turns into the
//! compile-time `pub const`s included below. Nothing here is parsed at runtime.

use std::path::Path;
use std::sync::OnceLock;

use crate::model::manifest::PnlManifest;

// Compile-time constants generated from `config.toml` by build.rs:
//   SCHEMA_VERSION, SELF_REPOSITORY, PACKAGES_REPOSITORY, DEFAULT_OUTPUT_DIR,
//   UPDATE_CHECK_TTL_SECONDS, UPDATE_CHECK_OPT_OUT_ENV, UPDATE_CHECK_CACHE_KEY,
//   BINARIES, AUTHORIZED_REPOSITORIES, and the `[filenames]` path constants
//   (PNL_MANIFEST_FILE, PNLX_MANIFEST_FILE, LOCK_FILE, PATHMAP_FILE,
//   AUTOLOAD_FILE, GENERATED_DIR, ALIASES_FILE, FFI_FILE_SUFFIX).
include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));

/// Whether an install-source URL is covered by a built-in authorized-repository
/// prefix (`repositories.authorized` in `config.toml`). Packages from these
/// trusted sources skip the "install scripts can run arbitrary commands" prompt.
pub fn is_authorized_repository(source_url: &str) -> bool {
    let source_url = source_url.trim();
    let normalized = normalize_git_source_url(source_url);

    AUTHORIZED_REPOSITORIES.iter().any(|prefix| {
        matches_authorized_prefix(source_url, prefix)
            || normalized
                .as_deref()
                .is_some_and(|source| matches_authorized_prefix(source, prefix))
    })
}

fn matches_authorized_prefix(source_url: &str, prefix: &str) -> bool {
    let Some(rest) = source_url.strip_prefix(prefix) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/') || rest.starts_with(".git")
}

fn normalize_git_source_url(source_url: &str) -> Option<String> {
    if let Some(rest) = source_url.strip_prefix("git@github.com:") {
        return Some(format!(
            "https://github.com/{}",
            strip_git_suffix_from_path(rest)
        ));
    }
    if let Some(rest) = source_url.strip_prefix("ssh://git@github.com/") {
        return Some(format!(
            "https://github.com/{}",
            strip_git_suffix_from_path(rest)
        ));
    }
    None
}

fn strip_git_suffix_from_path(path: &str) -> String {
    if let Some((repository, suffix)) = path.split_once(".git/") {
        format!("{repository}/{suffix}")
    } else {
        path.strip_suffix(".git").unwrap_or(path).to_owned()
    }
}

/// The default repository pnl releases come from.
pub fn default_self_repository() -> &'static str {
    SELF_REPOSITORY
}

/// The default package registry consulted for bare-name installs.
pub fn default_packages_repository() -> &'static str {
    PACKAGES_REPOSITORY
}

/// The self repository, honoring a `pnl.json` `config.release_repository` override
/// in the current directory. Resolved once and cached; falls back to the
/// built-in default when there is no workspace manifest or no override.
pub fn self_repository() -> &'static str {
    static RESOLVED: OnceLock<String> = OnceLock::new();
    RESOLVED
        .get_or_init(|| resolve_self_repository(Path::new(".")))
        .as_str()
}

fn resolve_self_repository(root: &Path) -> String {
    workspace_self_repository(root).unwrap_or_else(|| default_self_repository().to_owned())
}

/// Best-effort read of `config.release_repository` from `<root>/pnl.json`. Any
/// missing/unreadable/invalid manifest simply yields `None` (use the default).
fn workspace_self_repository(root: &Path) -> Option<String> {
    crate::util::io::read_json::<PnlManifest>(&root.join(crate::model::config::PNL_MANIFEST_FILE))
        .ok()?
        .config
        .release_repository
}

#[cfg(test)]
mod tests {
    use super::{
        default_packages_repository, default_self_repository, is_authorized_repository,
        resolve_self_repository,
    };

    #[test]
    fn embedded_config_parses_to_expected_defaults() {
        assert_eq!(default_self_repository(), "https://github.com/m3m0r7/pnl");
        assert_eq!(
            default_packages_repository(),
            "https://github.com/m3m0r7/pnl-packages/tree/main/packages"
        );
    }

    #[test]
    fn authorizes_first_party_package_sources() {
        // Both the direct-URL and bare-name (index-resolved) install URLs sit
        // under the whitelisted prefix, so they skip the install-script prompt.
        assert!(is_authorized_repository(
            "https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb"
        ));
        assert!(is_authorized_repository(
            "git@github.com:m3m0r7/pnl-packages.git"
        ));
        assert!(is_authorized_repository(
            "ssh://git@github.com/m3m0r7/pnl-packages.git"
        ));
        // An unrelated source still prompts.
        assert!(!is_authorized_repository(
            "https://github.com/someone-else/packages/tree/main/libusb"
        ));
        assert!(!is_authorized_repository(
            "https://github.com/m3m0r7/pnl-packages-fake/tree/main/packages/libusb"
        ));
        assert!(!is_authorized_repository("/local/path/to/package"));
    }

    #[test]
    fn self_repository_falls_back_when_no_workspace_manifest() {
        let dir = tempfile::tempdir().unwrap();
        // No pnl.json in this directory → the built-in default is used.
        assert_eq!(
            resolve_self_repository(dir.path()),
            default_self_repository()
        );
    }

    #[test]
    fn self_repository_honors_workspace_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::model::config::PNL_MANIFEST_FILE),
            r#"{
              "schema_version": "2026-07-01",
              "repositories": [],
              "library_paths": [],
              "config": {"release_repository": "https://example.test/fork"},
              "extensions": {}
            }"#,
        )
        .unwrap();
        assert_eq!(
            resolve_self_repository(dir.path()),
            "https://example.test/fork"
        );
    }
}
