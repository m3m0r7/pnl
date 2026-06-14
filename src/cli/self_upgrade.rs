//! `pnl self-upgrade`: check the pnl repository for a newer release tag, then
//! download, build, and install that release into a versioned layout.
//!
//! Layout (rooted at `--home`, `PNL_HOME`, or `$XDG_DATA_HOME/pnl`, which
//! defaults to `~/.local/share/pnl`):
//!
//! ```text
//! <home>/versions/<version>/bin/pnl
//! <home>/versions/<version>/bin/pnlx
//! <home>/current -> versions/<version>
//! <bin-dir>/pnl  -> <home>/current/bin/pnl    (default /usr/local/bin)
//! <bin-dir>/pnlx -> <home>/current/bin/pnlx
//! ```
//!
//! Keeping `<bin-dir>` entries as symlinks means an upgrade only swaps the
//! `current` link instead of overwriting the running executable in place.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use semver::Version;

use crate::config::BINARIES;

#[cfg(not(unix))]
pub fn self_upgrade(_bin_dir: &Path, _home: Option<&Path>) -> Result<()> {
    bail!(
        "pnl self-upgrade currently supports Unix only; download a release archive from {}/releases instead",
        crate::config::self_repository()
    )
}

#[cfg(unix)]
pub fn self_upgrade(bin_dir: &Path, home: Option<&Path>) -> Result<()> {
    crate::ui::heading("pnl", "self-upgrade");
    let started = std::time::Instant::now();

    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("failed to parse the running pnl version")?;
    let repository = crate::config::self_repository();

    crate::ui::step(&format!("fetching release tags from {repository}.git"));
    let Some(release) = crate::release::latest_live()? else {
        bail!("no release tags found at {repository}");
    };
    let (tag, latest) = (release.tag, release.version);

    if latest <= current {
        crate::ui::summary(&format!(
            "pnl {current} is already the latest release in {}",
            crate::ui::elapsed(started.elapsed())
        ));
        return Ok(());
    }

    // self-upgrade only manages the versioned symlink layout. A standalone
    // binary (downloaded and dropped on $PATH) cannot be swapped in place, so
    // point the user at the releases page instead of building a parallel layout.
    if crate::release::detect_install_kind() == crate::release::InstallKind::Standalone {
        crate::ui::warn(&format!(
            "pnl {latest} is available, but this binary was installed standalone; self-upgrade only updates the versioned symlink layout"
        ));
        crate::ui::info(&format!(
            "download pnl {latest} from {repository}/releases and reinstall it"
        ));
        return Ok(());
    }

    let layout = Layout::resolve(home)?;
    crate::ui::info(&format!(
        "upgrading {current} -> {latest} under {}",
        layout.home.display()
    ));

    let tarball_url = format!("{repository}/archive/refs/tags/{tag}.tar.gz");
    crate::ui::step(&format!("downloading {tarball_url}"));
    let tarball = crate::fetch::fetch_asset(&tarball_url)?;
    let source = extract_source_tarball(&tarball)?;

    crate::ui::step("building release binaries with cargo (this may take a while)");
    cargo_build(&source.source_root)?;

    let version_bin = layout.version_bin_dir(&latest);
    install_binaries(
        &source.source_root.join("target").join("release"),
        &version_bin,
    )?;
    crate::ui::created("installed", &version_bin);
    drop(source);

    switch_current(&layout, &latest)?;
    crate::ui::created("activated", &layout.current_link());

    link_binaries(bin_dir, &layout)?;

    crate::ui::summary(&format!(
        "upgraded pnl {current} -> {latest} in {}",
        crate::ui::elapsed(started.elapsed())
    ));
    Ok(())
}

/// Where versions, the `current` link, and the binaries live.
struct Layout {
    home: PathBuf,
}

impl Layout {
    /// `--home` when given; otherwise `PNL_HOME`, then the XDG base directory
    /// `$XDG_DATA_HOME/pnl` (`~/.local/share/pnl` when `XDG_DATA_HOME` is unset).
    fn resolve(home_override: Option<&Path>) -> Result<Self> {
        if let Some(home) = home_override {
            return Ok(Self {
                home: home.to_path_buf(),
            });
        }
        let home = default_home(
            std::env::var_os("PNL_HOME"),
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("HOME"),
        )?;
        Ok(Self { home })
    }

    fn version_bin_dir(&self, version: &Version) -> PathBuf {
        self.home
            .join("versions")
            .join(version.to_string())
            .join("bin")
    }

    fn current_link(&self) -> PathBuf {
        self.home.join("current")
    }

    fn current_bin(&self, name: &str) -> PathBuf {
        self.current_link().join("bin").join(name)
    }
}

/// The XDG-based default install root, from the raw environment values.
fn default_home(
    pnl_home: Option<std::ffi::OsString>,
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    if let Some(dir) = pnl_home {
        Ok(PathBuf::from(dir))
    } else if let Some(dir) = xdg_data_home {
        Ok(PathBuf::from(dir).join("pnl"))
    } else if let Some(dir) = home {
        Ok(PathBuf::from(dir).join(".local").join("share").join("pnl"))
    } else {
        bail!(
            "none of PNL_HOME, XDG_DATA_HOME, or HOME is set; pass --home to choose the install location"
        );
    }
}

/// A source tree extracted into a temporary directory, removed on drop.
struct ExtractedSource {
    root: PathBuf,
    /// The directory containing `Cargo.toml` (GitHub source tarballs wrap the
    /// tree in a single `<repo>-<version>/` directory).
    source_root: PathBuf,
}

impl Drop for ExtractedSource {
    fn drop(&mut self) {
        if self.root.exists() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn extract_source_tarball(tarball: &Path) -> Result<ExtractedSource> {
    let root = std::env::temp_dir().join(format!(
        "pnl-self-upgrade-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&root).with_context(|| format!("failed to create {}", root.display()))?;

    let file =
        fs::File::open(tarball).with_context(|| format!("failed to open {}", tarball.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in archive
        .entries()
        .with_context(|| format!("failed to read {}", tarball.display()))?
    {
        let mut entry = entry.with_context(|| format!("failed to read {}", tarball.display()))?;
        if !entry
            .unpack_in(&root)
            .with_context(|| format!("failed to extract {}", tarball.display()))?
        {
            bail!(
                "tar entry {} would extract outside the destination",
                entry.path()?.display()
            );
        }
    }

    let source_root = locate_cargo_root(&root)?;
    Ok(ExtractedSource { root, source_root })
}

/// Find the directory holding `Cargo.toml`: the extraction root, or a single
/// top-level subdirectory.
fn locate_cargo_root(destination: &Path) -> Result<PathBuf> {
    if destination.join("Cargo.toml").is_file() {
        return Ok(destination.to_path_buf());
    }
    for entry in fs::read_dir(destination)
        .with_context(|| format!("failed to read {}", destination.display()))?
    {
        let path = entry?.path();
        if path.is_dir() && path.join("Cargo.toml").is_file() {
            return Ok(path);
        }
    }
    bail!("the downloaded source archive does not contain Cargo.toml")
}

#[cfg_attr(not(unix), allow(dead_code))]
fn cargo_build(source_root: &Path) -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--bins", "--locked"])
        .current_dir(source_root)
        .status()
        .context(
            "failed to run cargo; self-upgrade builds from source and needs a Rust toolchain",
        )?;
    if !status.success() {
        bail!("cargo build failed with {status}");
    }
    Ok(())
}

#[cfg(unix)]
fn install_binaries(release_dir: &Path, version_bin: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(version_bin)
        .with_context(|| format!("failed to create {}", version_bin.display()))?;
    for name in BINARIES {
        let from = release_dir.join(name);
        let to = version_bin.join(name);
        fs::copy(&from, &to)
            .with_context(|| format!("failed to install {} to {}", from.display(), to.display()))?;
        fs::set_permissions(&to, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to set permissions on {}", to.display()))?;
    }
    Ok(())
}

/// Point `current` at `versions/<version>` through a staging link + rename, so
/// the `current` path never disappears mid-swap.
#[cfg(unix)]
fn switch_current(layout: &Layout, version: &Version) -> Result<()> {
    let link = layout.current_link();
    if let Ok(metadata) = link.symlink_metadata()
        && !metadata.file_type().is_symlink()
    {
        bail!(
            "{} already exists but is not a symlink; remove it and retry",
            link.display()
        );
    }

    let staging = layout.home.join(format!(".current-{}", std::process::id()));
    let _ = fs::remove_file(&staging);
    std::os::unix::fs::symlink(Path::new("versions").join(version.to_string()), &staging)
        .with_context(|| format!("failed to create {}", staging.display()))?;
    fs::rename(&staging, &link).with_context(|| format!("failed to update {}", link.display()))?;
    Ok(())
}

/// Ensure `<bin-dir>/pnl` and `<bin-dir>/pnlx` are symlinks into
/// `<home>/current/bin`, replacing plain binaries left by an old-style install.
#[cfg(unix)]
fn link_binaries(bin_dir: &Path, layout: &Layout) -> Result<()> {
    fs::create_dir_all(bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    for name in BINARIES {
        let link = bin_dir.join(name);
        let target = layout.current_bin(name);
        if fs::read_link(&link).is_ok_and(|existing| existing == target) {
            continue;
        }
        if link.symlink_metadata().is_ok() {
            fs::remove_file(&link).with_context(|| {
                format!(
                    "failed to replace {}; re-run with sudo or pass --bin-dir",
                    link.display()
                )
            })?;
        }
        std::os::unix::fs::symlink(&target, &link).with_context(|| {
            format!(
                "failed to link {}; re-run with sudo or pass --bin-dir",
                link.display()
            )
        })?;
        crate::ui::created("linked", &link);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_install_root_in_xdg_order() {
        let pnl_home = Some(std::ffi::OsString::from("/custom/pnl-home"));
        let xdg = Some(std::ffi::OsString::from("/xdg/data"));
        let home = Some(std::ffi::OsString::from("/home/user"));

        assert_eq!(
            default_home(pnl_home, xdg.clone(), home.clone()).unwrap(),
            PathBuf::from("/custom/pnl-home")
        );
        assert_eq!(
            default_home(None, xdg, home.clone()).unwrap(),
            PathBuf::from("/xdg/data/pnl")
        );
        assert_eq!(
            default_home(None, None, home).unwrap(),
            PathBuf::from("/home/user/.local/share/pnl")
        );
        assert!(default_home(None, None, None).is_err());
    }

    #[test]
    fn locates_cargo_root_in_wrapped_archive() {
        let dir = tempfile::tempdir().unwrap();
        let wrapped = dir.path().join("pnl-0.2.0");
        fs::create_dir_all(&wrapped).unwrap();
        fs::write(wrapped.join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(locate_cargo_root(dir.path()).unwrap(), wrapped);
    }
}
