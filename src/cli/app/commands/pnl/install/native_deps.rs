//! Making a package's native dependencies present: per-OS install recipes
//! (`setup.install`), the `/etc/os-release` key selection, and the `setup.build_script`
//! self-build path.

use std::path::Path;

use anyhow::{Context, Result};

use super::*;

/// The `installation` keys tried for the current platform, most specific
/// first. On Linux the distro ID from /etc/os-release (e.g. `alpine`,
/// `ubuntu`, `fedora`) is tried before each `ID_LIKE` ancestor (e.g. `debian`,
/// `rhel`) and the generic `linux` fallback.
fn installation_key_candidates() -> Vec<String> {
    match std::env::consts::OS {
        "macos" => vec!["darwin".to_owned()],
        "linux" => {
            let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            linux_installation_keys(&os_release)
        }
        other => vec![other.to_owned()],
    }
}

/// Candidate keys from /etc/os-release content: `ID`, then the `ID_LIKE`
/// tokens, then `linux`.
pub(super) fn linux_installation_keys(os_release: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut id_like = Vec::new();
    for line in os_release.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            keys.push(unquote_os_release(value).to_owned());
        } else if let Some(value) = line.strip_prefix("ID_LIKE=") {
            id_like = unquote_os_release(value)
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect();
        }
    }
    keys.extend(id_like);
    keys.push("linux".to_owned());
    keys.dedup();
    keys
}

fn unquote_os_release(value: &str) -> &str {
    value.trim().trim_matches(|ch| ch == '"' || ch == '\'')
}

/// Run a shell command line, inheriting stdio so the user sees its output.
fn run_shell(command_line: &str) -> std::io::Result<std::process::ExitStatus> {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C");
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg("-c");
        c
    };
    command.arg(command_line).status()
}

pub(super) fn run_self_build(extension_root: &Path, script: &str) -> Result<()> {
    let script =
        crate::native::install_script::resolve_package_relative_path(extension_root, script)?;
    crate::app::ui::step(&format!("running self_build: {}", script.display()));
    let status = if cfg!(windows) {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(&script)
            .current_dir(extension_root)
            .status()
    } else {
        std::process::Command::new("sh")
            .arg(&script)
            .current_dir(extension_root)
            .status()
    }
    .with_context(|| format!("failed to run self_build script {}", script.display()))?;

    if !status.success() {
        bail!("self_build script {} failed ({status})", script.display());
    }
    Ok(())
}

/// If the package declares `installation` for this platform, optionally run its
/// install commands. A passing `check_if_exists` short-circuits (already present);
/// otherwise the user confirms (auto-yes under `--yes`/`--no-interaction`).
pub(super) fn maybe_install_native_dependencies(
    extension: &PnlxManifest,
    interaction: &crate::app::interaction::Interaction,
) -> Result<()> {
    let candidates = installation_key_candidates();
    let Some(entry) = candidates
        .iter()
        .find_map(|key| extension.setup.install.get(key))
    else {
        return Ok(());
    };

    if !entry.check_if_exists.is_empty()
        && entry.check_if_exists.iter().all(|cmd| {
            run_shell(cmd)
                .map(|status| status.success())
                .unwrap_or(false)
        })
    {
        crate::app::ui::info(&format!(
            "{} native dependencies already present",
            extension.name
        ));
        return Ok(());
    }

    let listed = entry
        .commands
        .iter()
        .map(|cmd| format!("    {cmd}"))
        .collect::<Vec<_>>()
        .join("\n");
    let proceed = interaction.confirm(
        &format!(
            "{} needs native dependencies. Run the following to install them?\n{listed}\n",
            extension.name
        ),
        true,
    )?;
    if !proceed {
        crate::app::ui::warn("skipped native dependency installation; resolution may fail");
        return Ok(());
    }

    for cmd in &entry.commands {
        crate::app::ui::step(&format!("running: {cmd}"));
        let status =
            run_shell(cmd).with_context(|| format!("failed to run install command: {cmd}"))?;
        if !status.success() {
            bail!(
                "tried to install the native dependencies of {name} with the following command, but it failed ({status}):\n    {cmd}\n  install the required libraries and headers manually, then run `pnl install {name}` again",
                name = extension.name,
            );
        }
    }

    Ok(())
}
