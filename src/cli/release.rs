//! Discovering the latest pnl release and notifying about updates.
//!
//! `pnl`/`pnlx` check for a newer release on startup. The lookup goes over the
//! network (git `ls-remote`), so the result is cached for [`CHECK_TTL`]; until
//! the cache expires every run reuses it without touching the network.
//!
//! How an update is suggested depends on how this binary was installed:
//!
//! * a *managed* install (the versioned symlink layout `self-upgrade` and the
//!   `make install` target create) can be updated in place, so we point at
//!   `pnl self-upgrade`;
//! * a *standalone* binary (one downloaded from the releases page and dropped
//!   on `$PATH`) cannot be swapped safely, so we tell the user to download the
//!   new release and reinstall.

use std::ffi::OsStr;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

/// The repository pnl releases come from.
pub const SELF_REPOSITORY: &str = "https://github.com/m3m0r7/pnl";

/// How long a "latest release" lookup stays fresh before we re-check remotely.
pub const CHECK_TTL: Duration = Duration::from_secs(3600);

/// Cache key for the latest-release lookup under the shared cache.
const CACHE_KEY: &str = "latest-release";

/// Set to any value to skip the startup update check entirely.
const OPT_OUT_ENV: &str = "PNL_NO_UPDATE_CHECK";

/// A release tag (e.g. `v0.2.0`) and the version parsed from it.
#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub version: Version,
}

/// How this binary was installed, which decides how an update is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// The versioned symlink layout; `pnl self-upgrade` can update it.
    Managed,
    /// A standalone binary that must be re-downloaded to update.
    Standalone,
}

/// Print a one-line update notice to stderr when a newer release exists.
///
/// Best effort and silent on failure: it is skipped for non-interactive output
/// (so pipelines and CI stay clean) and when `PNL_NO_UPDATE_CHECK` is set, and
/// any network or cache error simply produces no notice.
pub fn notify_if_update_available() {
    if std::env::var_os(OPT_OUT_ENV).is_some() || !std::io::stderr().is_terminal() {
        return;
    }
    let Ok(current) = Version::parse(env!("CARGO_PKG_VERSION")) else {
        return;
    };
    let Ok(Some(latest)) = latest_cached_or_live(CHECK_TTL) else {
        return;
    };
    if latest.version <= current {
        return;
    }
    let upgrade_hint = match detect_install_kind() {
        InstallKind::Managed => "→ run `pnl self-upgrade` to update".to_owned(),
        InstallKind::Standalone => {
            format!("→ download it from {SELF_REPOSITORY}/releases and reinstall")
        }
    };
    crate::ui::notice_box(
        "✨ A new pnl release is available!",
        &[
            format!("📦 pnl {}  (you have {current})", latest.version),
            crate::ui::dim(&upgrade_hint),
        ],
    );
}

/// The latest release, served from the cache when it is still fresh and fetched
/// remotely (refreshing the cache) otherwise.
fn latest_cached_or_live(ttl: Duration) -> Result<Option<Release>> {
    if let Some(cached) = crate::cache::read_fresh::<CachedRelease>(CACHE_KEY, ttl)
        && let Ok(version) = Version::parse(&cached.version)
    {
        return Ok(Some(Release {
            tag: cached.tag,
            version,
        }));
    }
    latest_live()
}

/// Look up the latest release over the network and refresh the cache.
pub fn latest_live() -> Result<Option<Release>> {
    let tags = list_remote_tags(&format!("{SELF_REPOSITORY}.git"))?;
    let release = latest_release(&tags);
    if let Some(release) = &release {
        let _ = crate::cache::write(
            CACHE_KEY,
            &CachedRelease {
                tag: release.tag.clone(),
                version: release.version.to_string(),
            },
        );
    }
    Ok(release)
}

/// Detect whether the running binary lives in the managed symlink layout.
///
/// `self-upgrade`/`make install` place binaries at
/// `<home>/versions/<version>/bin/<name>`, which is what
/// [`std::env::current_exe`] resolves to (symlinks followed). Anything else —
/// a binary copied straight onto `$PATH`, or a `target/release` dev build — is
/// treated as standalone.
pub fn detect_install_kind() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::Standalone;
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if is_managed_layout(&exe) {
        InstallKind::Managed
    } else {
        InstallKind::Standalone
    }
}

/// True when `exe` matches `.../versions/<version>/bin/<name>`.
fn is_managed_layout(exe: &Path) -> bool {
    let mut ancestors = exe.ancestors();
    let _file = ancestors.next(); // .../bin/<name>
    let bin = ancestors.next(); // .../bin
    let version = ancestors.next(); // .../<version>
    let versions = ancestors.next(); // .../versions
    bin.and_then(Path::file_name) == Some(OsStr::new("bin"))
        && version.is_some()
        && versions.and_then(Path::file_name) == Some(OsStr::new("versions"))
}

/// The on-disk shape of the cached release; `Version` is stored as a string
/// because `semver::Version` does not derive serde without an extra feature.
#[derive(Serialize, Deserialize)]
struct CachedRelease {
    tag: String,
    version: String,
}

/// List tag names (e.g. `v0.2.0`) advertised by a remote repository, without
/// cloning it.
fn list_remote_tags(url: &str) -> Result<Vec<String>> {
    let mut remote = git2::Remote::create_detached(url)
        .with_context(|| format!("failed to create a remote for {url}"))?;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(crate::git_source::authenticate);
    let connection = remote
        .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
        .with_context(|| format!("failed to connect to {url}"))?;
    let tags = connection
        .list()
        .with_context(|| format!("failed to list references of {url}"))?
        .iter()
        .filter_map(|head| head.name().strip_prefix("refs/tags/"))
        .filter(|name| !name.ends_with("^{}"))
        .map(ToOwned::to_owned)
        .collect();
    Ok(tags)
}

/// The highest release among the tags, with its original tag name.
fn latest_release(tags: &[String]) -> Option<Release> {
    tags.iter()
        .filter_map(|tag| {
            Some(Release {
                tag: tag.clone(),
                version: release_version_from_tag(tag)?,
            })
        })
        .max_by(|left, right| left.version.cmp(&right.version))
}

/// Parse `v1.2.3` / `1.2.3` tags; pre-releases are not auto-upgrade targets.
fn release_version_from_tag(tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    version.pre.is_empty().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn picks_the_highest_release_tag() {
        let tags = vec![
            "v0.1.2".to_owned(),
            "v0.10.0".to_owned(),
            "v0.2.0".to_owned(),
            "v1.0.0-alpha.1".to_owned(),
            "not-a-version".to_owned(),
        ];
        let release = latest_release(&tags).unwrap();
        assert_eq!(release.tag, "v0.10.0");
        assert_eq!(release.version, Version::new(0, 10, 0));
    }

    #[test]
    fn ignores_repositories_without_release_tags() {
        assert!(latest_release(&["main".to_owned(), "v1.0.0-rc.1".to_owned()]).is_none());
    }

    #[test]
    fn parses_release_tags_only() {
        assert_eq!(
            release_version_from_tag("v1.2.3"),
            Some(Version::new(1, 2, 3))
        );
        assert_eq!(
            release_version_from_tag("1.2.3"),
            Some(Version::new(1, 2, 3))
        );
        assert_eq!(release_version_from_tag("v1.2.3-rc.1"), None);
        assert_eq!(release_version_from_tag("release-1"), None);
    }

    #[test]
    fn recognizes_the_managed_symlink_layout() {
        assert!(is_managed_layout(&PathBuf::from(
            "/home/u/.local/share/pnl/versions/0.2.0/bin/pnl"
        )));
        assert!(is_managed_layout(&PathBuf::from(
            "/opt/pnl/versions/1.0.0/bin/pnlx"
        )));
    }

    #[test]
    fn rejects_standalone_binary_paths() {
        assert!(!is_managed_layout(&PathBuf::from("/usr/local/bin/pnl")));
        assert!(!is_managed_layout(&PathBuf::from(
            "/Volumes/develop/m3m0r7/pnl/target/release/pnl"
        )));
    }
}
