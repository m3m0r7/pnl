use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;

/// Whether a target looks like a distributable archive (by file extension). The
/// query string of a URL is ignored, so `…/pkg.tar.gz?token=…` still matches.
pub fn is_archive_source(target: &str) -> bool {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar")
        || lower.ends_with(".zip")
}

/// Extract an archive into a fresh temporary directory and return the directory
/// that contains its `pnlx.json` (the archive root, or a single wrapping
/// subdirectory). Errors if the archive holds no `pnlx.json`.
pub fn extract_extension_archive(archive: &Path) -> Result<PathBuf> {
    let destination = unique_temp_dir(archive);
    std::fs::create_dir_all(&destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;

    let lower = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if lower.ends_with(".zip") {
        extract_zip(archive, &destination)?;
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let file =
            File::open(archive).with_context(|| format!("failed to open {}", archive.display()))?;
        tar::Archive::new(GzDecoder::new(file))
            .unpack(&destination)
            .with_context(|| format!("failed to extract {}", archive.display()))?;
    } else {
        let file =
            File::open(archive).with_context(|| format!("failed to open {}", archive.display()))?;
        tar::Archive::new(file)
            .unpack(&destination)
            .with_context(|| format!("failed to extract {}", archive.display()))?;
    }

    locate_extension_root(&destination)
        .with_context(|| format!("archive {} does not contain a pnlx.json", archive.display()))
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("failed to open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip {}", archive.display()))?;
    zip.extract(destination)
        .with_context(|| format!("failed to extract {}", archive.display()))?;
    Ok(())
}

/// Find the directory holding `pnlx.json`: the extraction root, or a single
/// top-level subdirectory (archives commonly wrap their contents in one folder).
fn locate_extension_root(destination: &Path) -> Result<PathBuf> {
    if destination.join("pnlx.json").is_file() {
        return Ok(destination.to_path_buf());
    }

    for entry in std::fs::read_dir(destination)
        .with_context(|| format!("failed to read {}", destination.display()))?
    {
        let path = entry?.path();
        if path.is_dir() && path.join("pnlx.json").is_file() {
            return Ok(path);
        }
    }

    bail!("no pnlx.json found after extraction")
}

fn unique_temp_dir(archive: &Path) -> PathBuf {
    let stem = archive
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("archive");
    std::env::temp_dir().join(format!(
        "pnl-archive-{stem}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::is_archive_source;

    #[test]
    fn recognizes_archive_sources() {
        assert!(is_archive_source("pkg.tar.gz"));
        assert!(is_archive_source("pkg.tgz"));
        assert!(is_archive_source("pkg.zip"));
        assert!(is_archive_source(
            "https://example.com/a/pkg.tar.gz?token=1"
        ));
        assert!(!is_archive_source("https://github.com/o/r"));
        assert!(!is_archive_source("./local/pkg"));
    }
}
