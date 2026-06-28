//! Filesystem-path helpers shared across layers.

use std::path::{Path, PathBuf};

/// Resolve `path` against `root` when it is relative; leave it untouched when it
/// is already absolute.
pub fn absolutize(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
