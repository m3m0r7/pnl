//! The PHP SDK, embedded into the binary so `pnl install` can write a
//! self-contained, dependency-free copy into `@pnlx/runtime/`.
//!
//! The whole `src/sdk` tree is embedded at build time and walked at install
//! time, so adding a file under `src/sdk` needs no change here.

use include_dir::{Dir, include_dir};

/// The entire PHP SDK tree (`src/sdk`), embedded verbatim. Each file's path is
/// relative to `src/sdk`, mirrored under `@pnlx/runtime/` on install.
pub static SDK_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/src/sdk");

/// A content fingerprint of `src/sdk`, produced by `build.rs`. `include_dir!`
/// does not register the embedded files as rebuild triggers on stable Rust, so
/// `include_str!`ing this build-generated fingerprint (which cargo DOES track)
/// forces this module — and therefore the `include_dir!` embed above — to be
/// recompiled whenever any SDK file changes. The value is intentionally unused.
const _SDK_FINGERPRINT: &str = include_str!(concat!(env!("OUT_DIR"), "/sdk_fingerprint.txt"));
