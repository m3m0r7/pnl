//! The PHP SDK, embedded into the binary so `pnl install` can write a
//! self-contained, dependency-free copy into `@pnlx/runtime/`.
//!
//! The whole `src/sdk` tree is embedded at build time and walked at install
//! time, so adding a file under `src/sdk` needs no change here.

use include_dir::{Dir, include_dir};

/// The entire PHP SDK tree (`src/sdk`), embedded verbatim. Each file's path is
/// relative to `src/sdk`, mirrored under `@pnlx/runtime/` on install.
pub static SDK_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/src/sdk");
