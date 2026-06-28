//! Fetching remote headers and native libraries.
//!
//! Sources may be local (handled by the caller) or remote over http(s), ftp, or
//! git. Downloads are cached under the user cache directory keyed by URL so
//! repeated installs do not re-download. `$PATH`-style local discovery remains
//! the fallback when no remote source is given.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{FtpStream, NativeTlsConnector, NativeTlsFtpStream};

use crate::sources::git_source::{GitSource, install_git_source};

/// Returns true when `source` is a remote URL this module can fetch, as opposed
/// to a local file path.
pub fn is_remote_source(source: &str) -> bool {
    SourceKind::classify(source).is_some()
}

/// Fetch a remote asset to the local cache and return the cached file path.
pub fn fetch_asset(url: &str) -> Result<PathBuf> {
    let kind = SourceKind::classify(url)
        .with_context(|| format!("unsupported remote source URL: {url}"))?;

    let destination = cache_path(url);
    if destination.is_file() {
        return Ok(destination);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }

    let temp = destination.with_extension(format!("{}.tmp", std::process::id()));
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }

    match kind {
        SourceKind::Http => fetch_http(url, &temp)?,
        SourceKind::Ftp => fetch_ftp(url, &temp, false)?,
        SourceKind::Ftps => fetch_ftp(url, &temp, true)?,
        SourceKind::Git => fetch_git(url, &temp)?,
    }
    fs::rename(&temp, &destination)
        .with_context(|| format!("failed to move fetched asset to {}", destination.display()))?;

    Ok(destination)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Http,
    Ftp,
    Ftps,
    Git,
}

impl SourceKind {
    fn classify(url: &str) -> Option<Self> {
        if url.starts_with("ftps://") {
            Some(Self::Ftps)
        } else if url.starts_with("ftp://") {
            Some(Self::Ftp)
        } else if url.starts_with("git@") || url.starts_with("git://") || url.starts_with("ssh://")
        {
            Some(Self::Git)
        } else if url.starts_with("http://") || url.starts_with("https://") {
            // A browse URL into a repo tree, or an explicit `.git`, is cloned;
            // anything else (release assets, raw files) is downloaded directly.
            if url.ends_with(".git") || url.contains("/tree/") || url.contains("/-/tree/") {
                Some(Self::Git)
            } else {
                Some(Self::Http)
            }
        } else {
            None
        }
    }
}

fn fetch_http(url: &str, destination: &Path) -> Result<()> {
    let response = ureq::get(url)
        .call()
        .with_context(|| format!("failed to download {url}"))?;
    if !response.status().is_success() {
        bail!("download of {url} failed with status {}", response.status());
    }

    let mut reader = response.into_body().into_reader();
    let mut file = File::create(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    io::copy(&mut reader, &mut file).with_context(|| format!("failed to save {url}"))?;
    Ok(())
}

/// Fetch over FTP, optionally upgrading the control connection to TLS (FTPS,
/// explicit `AUTH TLS`) when `secure` is set.
fn fetch_ftp(url: &str, destination: &Path, secure: bool) -> Result<()> {
    let parsed = url::Url::parse(url).with_context(|| format!("invalid ftp URL: {url}"))?;
    let host = parsed.host_str().context("ftp URL is missing a host")?;
    let port = parsed.port().unwrap_or(21);
    let user = if parsed.username().is_empty() {
        "anonymous"
    } else {
        parsed.username()
    };
    let password = parsed.password().unwrap_or("anonymous@");

    // The plain and TLS-upgraded streams are distinct types, so each branch runs
    // the same login/retrieve sequence and yields the downloaded bytes.
    let path = parsed.path();
    let data = if secure {
        let connector = NativeTlsConnector::from(
            TlsConnector::new().context("failed to initialize the TLS connector for ftps")?,
        );
        let mut stream = NativeTlsFtpStream::connect((host, port))
            .with_context(|| format!("failed to connect to {host}"))?
            .into_secure(connector, host)
            .with_context(|| format!("failed to start a TLS session with {host}"))?;
        stream
            .login(user, password)
            .with_context(|| format!("failed to log in to {host}"))?;
        stream
            .transfer_type(FileType::Binary)
            .context("failed to switch to binary transfer mode")?;
        let data = stream
            .retr_as_buffer(path)
            .with_context(|| format!("failed to retrieve {path}"))?;
        let _ = stream.quit();
        data
    } else {
        let mut stream = FtpStream::connect((host, port))
            .with_context(|| format!("failed to connect to {host}"))?;
        stream
            .login(user, password)
            .with_context(|| format!("failed to log in to {host}"))?;
        stream
            .transfer_type(FileType::Binary)
            .context("failed to switch to binary transfer mode")?;
        let data = stream
            .retr_as_buffer(path)
            .with_context(|| format!("failed to retrieve {path}"))?;
        let _ = stream.quit();
        data
    };

    fs::write(destination, data.into_inner()).with_context(|| format!("failed to save {url}"))?;
    Ok(())
}

fn fetch_git(url: &str, destination: &Path) -> Result<()> {
    let source = GitSource::parse(url)?;
    if source.package_path.as_os_str().is_empty() {
        bail!(
            "git asset URL must point to a file inside the repository, e.g. a /tree/<branch>/<path> URL: {url}"
        );
    }

    let installed = install_git_source(&source)?;
    let file = installed.destination.join(&source.package_path);
    if !file.is_file() {
        bail!(
            "{} does not point to a file in {}",
            source.package_path.display(),
            source.url
        );
    }
    fs::copy(&file, destination)
        .with_context(|| format!("failed to copy fetched file to {}", destination.display()))?;
    Ok(())
}

fn cache_path(url: &str) -> PathBuf {
    cache_root()
        .join("remote")
        .join(short_hash(url))
        .join(file_name_from_url(url))
}

fn cache_root() -> PathBuf {
    // A subdirectory of the shared cache root so `pnl purge cache` clears it.
    crate::sources::cache::root().join("fetch")
}

fn file_name_from_url(url: &str) -> String {
    let trimmed = url.split(['?', '#']).next().unwrap_or(url);
    let name = trimmed.rsplit(['/', ':']).next().unwrap_or("");
    let name = name.trim_end_matches(".git");
    if name.is_empty() {
        "asset".to_owned()
    } else {
        name.to_owned()
    }
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_source_urls() {
        assert_eq!(
            SourceKind::classify("https://example.com/lib.so"),
            Some(SourceKind::Http)
        );
        assert_eq!(
            SourceKind::classify("https://raw.githubusercontent.com/o/r/main/foo.h"),
            Some(SourceKind::Http)
        );
        assert_eq!(
            SourceKind::classify("ftp://example.com/lib.so"),
            Some(SourceKind::Ftp)
        );
        assert_eq!(
            SourceKind::classify("ftps://example.com/lib.so"),
            Some(SourceKind::Ftps)
        );
        assert_eq!(
            SourceKind::classify("git@github.com:o/r.git"),
            Some(SourceKind::Git)
        );
        assert_eq!(
            SourceKind::classify("https://github.com/o/r.git"),
            Some(SourceKind::Git)
        );
        assert_eq!(
            SourceKind::classify("https://github.com/o/r/tree/main/include/foo.h"),
            Some(SourceKind::Git)
        );
        assert_eq!(SourceKind::classify("/usr/local/lib/libfoo.so"), None);
        assert!(!is_remote_source("/usr/local/lib/libfoo.so"));
        assert!(is_remote_source("https://example.com/lib.so"));
    }

    #[test]
    fn derives_cache_file_names() {
        assert_eq!(
            file_name_from_url("https://example.com/path/libfoo.so"),
            "libfoo.so"
        );
        assert_eq!(
            file_name_from_url("https://example.com/libfoo.so?v=1"),
            "libfoo.so"
        );
        assert_eq!(file_name_from_url("git@github.com:o/r.git"), "r");
    }
}
