//! Resolving the generated-workspace directory (`@pnlx` by default).
//!
//! The directory name comes from `output_dir` in the project's `pnl.json`, so
//! all the generated artifacts (lock, pathmap, installed packages, autoload)
//! live under a single configurable location.

use std::path::{Path, PathBuf};

use crate::io::read_json;
use crate::manifest::PnlManifest;

pub use crate::config::DEFAULT_OUTPUT_DIR;

/// The absolute path of the generated workspace directory under `root`.
pub fn workspace_dir(root: &Path) -> PathBuf {
    root.join(output_dir_name(root))
}

/// The configured workspace directory name for the project at `root`, falling
/// back to the default when no (valid) `pnl.json` is present.
pub fn output_dir_name(root: &Path) -> String {
    read_json::<PnlManifest>(&root.join(crate::config::PNL_MANIFEST_FILE))
        .ok()
        .map(|manifest| manifest.output_dir)
        .filter(|dir| !dir.is_empty())
        .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::write_json;
    use crate::manifest::PnlManifest;

    #[test]
    fn defaults_to_at_pnlx_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(output_dir_name(dir.path()), "@pnlx");
        assert_eq!(workspace_dir(dir.path()), dir.path().join("@pnlx"));
    }

    #[test]
    fn honors_configured_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = PnlManifest {
            output_dir: "build/workspace".to_owned(),
            ..PnlManifest::default()
        };
        write_json(
            &dir.path().join(crate::config::PNL_MANIFEST_FILE),
            &manifest,
        )
        .unwrap();

        assert_eq!(output_dir_name(dir.path()), "build/workspace");
        assert_eq!(
            workspace_dir(dir.path()),
            dir.path().join("build/workspace")
        );
    }
}
