use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use git_url_parse::GitUrl;
use git2::build::RepoBuilder;
use git2::{
    Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository, SubmoduleUpdateOptions,
};

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
        if input.starts_with("git@") {
            return Self::from_scp_like_git(input);
        }

        let url = parse_git_url(input)?;
        match url.scheme() {
            // Web (browser) URLs carry host-specific `tree`/`src` conventions.
            Some("http" | "https") => Self::from_web_url(&url),
            // Plain transport URLs are cloned verbatim.
            Some("ssh" | "git") => Self::from_generic_git_url(input, &url),
            _ if input.ends_with(".git") => Self::from_generic_git_url(input, &url),
            _ => bail!("unsupported git source: {input}"),
        }
    }

    pub fn package_name(&self) -> String {
        format!("{}/{}", self.vendor, self.name)
    }

    fn from_web_url(url: &GitUrl) -> Result<Self> {
        let scheme = url.scheme().unwrap_or("https");
        let host = url.host().context("web URL must include a host")?;
        let parts = path_segments(url.path());
        let (vendor, name, branch, package_path) = match HostKind::of(host) {
            HostKind::GitHub => split_github(&parts)?,
            HostKind::GitLab => split_gitlab(&parts)?,
            HostKind::Bitbucket => split_bitbucket(&parts)?,
            HostKind::Generic => split_generic_web(&parts)?,
        };

        Ok(Self {
            url: format!("{scheme}://{host}/{vendor}/{name}.git"),
            vendor: sanitize_package_segment(vendor)?,
            name: sanitize_package_segment(name)?,
            branch,
            package_path,
        })
    }

    fn from_scp_like_git(input: &str) -> Result<Self> {
        let url = parse_git_url(input)?;
        let host = url
            .host()
            .context("scp-like git source must include a host")?;
        let user = url.user().unwrap_or("git");
        let parts = path_segments(url.path());
        let (vendor, name, branch, package_path) = split_github(&parts)?;

        Ok(Self {
            url: format!("{user}@{host}:{vendor}/{name}.git"),
            vendor: sanitize_package_segment(vendor)?,
            name: sanitize_package_segment(name)?,
            branch,
            package_path,
        })
    }

    fn from_generic_git_url(input: &str, url: &GitUrl) -> Result<Self> {
        let parts = path_segments(url.path());
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

/// Known forges whose browser URLs embed a branch/sub-path convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostKind {
    GitHub,
    GitLab,
    Bitbucket,
    Generic,
}

impl HostKind {
    fn of(host: &str) -> Self {
        let host = host.to_ascii_lowercase();
        if host == "github.com" || host.ends_with(".github.com") {
            Self::GitHub
        } else if host == "gitlab.com" || host.contains("gitlab") {
            Self::GitLab
        } else if host == "bitbucket.org" || host.contains("bitbucket") {
            Self::Bitbucket
        } else {
            Self::Generic
        }
    }
}

fn parse_git_url(input: &str) -> Result<GitUrl> {
    GitUrl::parse(input).map_err(|err| anyhow!("unsupported git source: {input} ({err})"))
}

fn path_segments(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .trim_end_matches(".git")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

type RepoParts<'a> = (&'a str, &'a str, Option<String>, PathBuf);

fn owner_and_repo<'a>(parts: &'a [&'a str]) -> Result<(&'a str, &'a str, &'a [&'a str])> {
    let vendor = parts.first().copied().context("missing git owner")?;
    let raw_name = parts
        .get(1)
        .copied()
        .context("missing git repository name")?;
    Ok((vendor, raw_name.trim_end_matches(".git"), &parts[2..]))
}

/// `owner/repo[/tree/<branch>[/<sub-path>]]` — also used for scp-like paths.
fn split_github<'a>(parts: &'a [&'a str]) -> Result<RepoParts<'a>> {
    let (vendor, name, rest) = owner_and_repo(parts)?;
    let (branch, package_parts) = match rest.first().copied() {
        Some("tree") => (Some(require_branch(rest)?), &rest[2..]),
        Some("blob" | "commit" | "releases") => {
            bail!("install source must point to a repository or package directory")
        }
        _ => (None, rest),
    };
    Ok((vendor, name, branch, package_parts.iter().collect()))
}

/// `owner/repo[/-/tree/<branch>[/<sub-path>]]` (GitLab web URLs).
fn split_gitlab<'a>(parts: &'a [&'a str]) -> Result<RepoParts<'a>> {
    let (vendor, name, mut rest) = owner_and_repo(parts)?;
    if rest.first() == Some(&"-") {
        rest = &rest[1..];
    }
    let (branch, package_parts) = match rest.first().copied() {
        Some("tree") => (Some(require_branch(rest)?), &rest[2..]),
        Some("blob" | "commit" | "merge_requests" | "issues") => {
            bail!("install source must point to a repository or package directory")
        }
        _ => (None, rest),
    };
    Ok((vendor, name, branch, package_parts.iter().collect()))
}

/// `owner/repo[/src/<branch>[/<sub-path>]]` (Bitbucket web URLs).
fn split_bitbucket<'a>(parts: &'a [&'a str]) -> Result<RepoParts<'a>> {
    let (vendor, name, rest) = owner_and_repo(parts)?;
    let (branch, package_parts) = match rest.first().copied() {
        Some("src") => (Some(require_branch(rest)?), &rest[2..]),
        Some("commits" | "branches" | "pull-requests" | "downloads") => {
            bail!("install source must point to a repository or package directory")
        }
        _ => (None, rest),
    };
    Ok((vendor, name, branch, package_parts.iter().collect()))
}

/// Unknown hosts: `owner/repo[/<sub-path>]`, with no branch convention.
fn split_generic_web<'a>(parts: &'a [&'a str]) -> Result<RepoParts<'a>> {
    let (vendor, name, rest) = owner_and_repo(parts)?;
    Ok((vendor, name, None, rest.iter().collect()))
}

/// The segment after a `tree`/`src` marker is the branch and must be present.
fn require_branch(rest: &[&str]) -> Result<String> {
    rest.get(1)
        .copied()
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned)
        .context("tree URL must include a branch name")
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

    let mut builder = RepoBuilder::new();
    builder.fetch_options(fetch_options());
    if let Some(branch) = &source.branch {
        builder.branch(branch);
    }

    let repo = builder
        .clone(&source.url, &tmp_dir)
        .with_context(|| format!("failed to clone {}", source.url))?;
    update_submodules(&repo)?;

    let revision = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .context("failed to resolve cloned HEAD commit")?
        .id()
        .to_string();

    Ok(InstalledSource {
        revision,
        destination: tmp_dir,
    })
}

/// Recursively initialise and update submodules, mirroring `git clone --recursive`.
fn update_submodules(repo: &Repository) -> Result<()> {
    for mut submodule in repo.submodules()? {
        let mut options = SubmoduleUpdateOptions::new();
        options.fetch(fetch_options());
        submodule
            .update(true, Some(&mut options))
            .with_context(|| format!("failed to update submodule {:?}", submodule.name()))?;
        let nested = submodule.open()?;
        update_submodules(&nested)?;
    }
    Ok(())
}

/// Shallow fetch (`--depth 1`) with credential-helper / ssh-agent authentication,
/// matching the behaviour the `git` CLI provided out of the box.
fn fetch_options<'cb>() -> FetchOptions<'cb> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(authenticate);

    let mut options = FetchOptions::new();
    options.depth(1);
    options.remote_callbacks(callbacks);
    options
}

fn authenticate(
    url: &str,
    username: Option<&str>,
    allowed: CredentialType,
) -> std::result::Result<Cred, git2::Error> {
    if allowed.contains(CredentialType::SSH_KEY) {
        return Cred::ssh_key_from_agent(username.unwrap_or("git"));
    }
    if allowed.contains(CredentialType::USER_PASS_PLAINTEXT)
        && let Ok(config) = git2::Config::open_default()
        && let Ok(cred) = Cred::credential_helper(&config, url, username)
    {
        return Ok(cred);
    }
    Cred::default()
}
