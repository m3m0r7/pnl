use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use crate::validate::sanitize_package_segment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    pub vendor: String,
    pub name: String,
    pub branch: Option<String>,
    pub package_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InstalledSource {
    pub revision: String,
    pub destination: PathBuf,
}

impl Drop for InstalledSource {
    fn drop(&mut self) {
        if self.destination.exists() {
            let _ = std::fs::remove_dir_all(&self.destination);
        }
    }
}

impl GitSource {
    pub fn parse(input: &str) -> Result<Self> {
        if input.starts_with("https://github.com/") || input.starts_with("http://github.com/") {
            return Self::from_github_https(input);
        }

        if input.starts_with("git@") {
            return Self::from_scp_like_git(input);
        }

        if input.starts_with("ssh://") || input.starts_with("git://") || input.ends_with(".git") {
            return Self::from_generic_git_url(input);
        }

        bail!("unsupported git source: {input}");
    }

    pub fn package_name(&self) -> String {
        format!("{}/{}", self.vendor, self.name)
    }

    fn from_github_https(input: &str) -> Result<Self> {
        let (scheme, rest) = input
            .split_once("://")
            .context("GitHub source must include a URL scheme")?;
        let trimmed = rest
            .trim_start_matches("github.com/")
            .trim_end_matches('/')
            .trim_end_matches(".git");
        let parts = trimmed
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let (vendor, name, branch, package_path) = split_repo_and_package_path(&parts)?;

        Ok(Self {
            url: format!("{scheme}://github.com/{vendor}/{name}.git"),
            vendor: sanitize_package_segment(vendor)?,
            name: sanitize_package_segment(name)?,
            branch,
            package_path,
        })
    }

    fn from_scp_like_git(input: &str) -> Result<Self> {
        let (host, path) = input
            .split_once(':')
            .context("scp-like git source must include ':' before repository path")?;
        let path = path.trim_matches('/').trim_end_matches(".git");
        let parts = path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let (vendor, name, branch, package_path) = split_repo_and_package_path(&parts)?;

        Ok(Self {
            url: format!("{host}:{vendor}/{name}.git"),
            vendor: sanitize_package_segment(vendor)?,
            name: sanitize_package_segment(name)?,
            branch,
            package_path,
        })
    }

    fn from_generic_git_url(input: &str) -> Result<Self> {
        let path = git_path_part(input).context("git source must include a repository path")?;
        let trimmed = path.trim_matches('/').trim_end_matches(".git");
        let parts = trimmed
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let name = parts
            .last()
            .copied()
            .filter(|value| !value.is_empty())
            .context("missing git repository name")?;
        let vendor = parts.iter().rev().nth(1).copied().unwrap_or("git-source");

        Ok(Self {
            url: input.to_owned(),
            vendor: sanitize_package_segment(vendor)?,
            name: sanitize_package_segment(name)?,
            branch: None,
            package_path: PathBuf::new(),
        })
    }
}

fn split_repo_and_package_path<'a>(
    parts: &'a [&'a str],
) -> Result<(&'a str, &'a str, Option<String>, PathBuf)> {
    let vendor = parts.first().copied().context("missing git owner")?;
    let raw_name = parts
        .get(1)
        .copied()
        .context("missing git repository name")?;
    let name = raw_name.trim_end_matches(".git");
    let package_parts = &parts[2..];
    let (branch, package_parts) = match package_parts.first().copied() {
        Some("tree") => {
            let branch = package_parts
                .get(1)
                .copied()
                .context("GitHub tree URL must include a branch name")?;
            (Some(branch.to_owned()), &package_parts[2..])
        }
        Some("blob" | "commit" | "releases") => {
            bail!("GitHub install source must point to a repository or package directory")
        }
        _ => (None, package_parts),
    };
    if branch.as_deref().is_some_and(str::is_empty) {
        bail!("GitHub tree URL must include a branch name");
    }
    Ok((vendor, name, branch, package_parts.iter().collect()))
}

fn git_path_part(input: &str) -> Option<&str> {
    if let Some((_, path)) = input.split_once("://") {
        return path.split_once('/').map(|(_, path)| path);
    }

    if input.starts_with("git@") {
        return input.split_once(':').map(|(_, path)| path);
    }

    input.rsplit_once('/').map(|(_, path)| path)
}

pub fn install_git_source(source: &GitSource) -> Result<InstalledSource> {
    let tmp_root = std::env::temp_dir().join("pnl").join("git");
    std::fs::create_dir_all(&tmp_root)
        .with_context(|| format!("failed to create {}", tmp_root.display()))?;
    let tmp_dir = tmp_root.join(format!(
        "{}-{}-{}",
        source.name,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)
            .with_context(|| format!("failed to clear {}", tmp_dir.display()))?;
    }

    let mut args = vec![
        "clone".to_owned(),
        "--depth".to_owned(),
        "1".to_owned(),
        "--recursive".to_owned(),
    ];
    if let Some(branch) = &source.branch {
        args.push("--branch".to_owned());
        args.push(branch.clone());
    }
    args.push(source.url.clone());
    args.push(
        tmp_dir
            .to_str()
            .context("temporary path is not valid UTF-8")?
            .to_owned(),
    );
    run_git(&args)?;

    let revision = git_output(&tmp_dir, ["rev-parse", "HEAD"])?;
    Ok(InstalledSource {
        revision,
        destination: tmp_dir,
    })
}

fn run_git(args: &[String]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .status()
        .context("failed to start git")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("git exited with status {status}"))
    }
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .context("failed to start git")?;
    if !output.status.success() {
        bail!("git exited with status {}", output.status);
    }
    Ok(String::from_utf8(output.stdout)
        .context("git output was not UTF-8")?
        .trim()
        .to_owned())
}
