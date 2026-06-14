//! Built-in configuration, baked in from `config.toml` at build time and (for
//! the repository endpoints) overridable per project via the `config` block in
//! `pnl.json`.
//!
//! The hardcoded constants used to live scattered across the crate; they are now
//! a single source of truth in `config.toml`, which `build.rs` turns into the
//! compile-time `pub const`s included below. Nothing here is parsed at runtime.

use std::path::Path;
use std::sync::OnceLock;

use crate::manifest::PnlManifest;

// Compile-time constants generated from `config.toml` by build.rs:
//   SCHEMA_VERSION, SELF_REPOSITORY, PACKAGES_REPOSITORY, DEFAULT_OUTPUT_DIR,
//   UPDATE_CHECK_TTL_SECONDS, UPDATE_CHECK_OPT_OUT_ENV, UPDATE_CHECK_CACHE_KEY,
//   BINARIES.
include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));

/// The default repository pnl releases come from.
pub fn default_self_repository() -> &'static str {
    SELF_REPOSITORY
}

/// The default package registry consulted for bare-name installs.
pub fn default_packages_repository() -> &'static str {
    PACKAGES_REPOSITORY
}

/// The self repository, honoring a `pnl.json` `config.self_repository` override
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

/// Best-effort read of `config.self_repository` from `<root>/pnl.json`. Any
/// missing/unreadable/invalid manifest simply yields `None` (use the default).
fn workspace_self_repository(root: &Path) -> Option<String> {
    crate::io::read_json::<PnlManifest>(&root.join("pnl.json"))
        .ok()?
        .config
        .self_repository
}

#[cfg(test)]
mod tests {
    use super::{default_packages_repository, default_self_repository, resolve_self_repository};

    #[test]
    fn embedded_config_parses_to_expected_defaults() {
        assert_eq!(default_self_repository(), "https://github.com/m3m0r7/pnl");
        assert_eq!(
            default_packages_repository(),
            "https://github.com/m3m0r7/pnl-packages/tree/main/packages"
        );
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
            dir.path().join("pnl.json"),
            r#"{
              "schema_version": "2026-07-01",
              "repositories": [],
              "load_paths": [],
              "config": {"self_repository": "https://example.test/fork"},
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
