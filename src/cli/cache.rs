//! Shared on-disk cache rooted at the XDG cache directory.
//!
//! Everything pnl caches between runs lives under a single root
//! (`$XDG_CACHE_HOME/pnl`, falling back to `~/.cache/pnl`), so a single
//! `pnl purge cache` can clear it all. New caches should be subdirectories of
//! [`root`] rather than scattered elsewhere:
//!
//! ```text
//! <root>/fetch/   downloaded headers and native libraries (see fetch.rs)
//! <root>/state/   short-lived JSON values with a TTL (see read_fresh/write)
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The root of every pnl cache: `$XDG_CACHE_HOME/pnl`, then `~/.cache/pnl`,
/// then a temp-dir fallback when neither is set.
pub fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(dir).join("pnl")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join("pnl")
    } else {
        std::env::temp_dir().join("pnl")
    }
}

/// Remove the entire cache tree. Returns `true` when something was removed.
pub fn purge() -> Result<bool> {
    let root = root();
    if !root.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&root).with_context(|| format!("failed to remove {}", root.display()))?;
    Ok(true)
}

/// A cached value with the wall-clock time it was stored, used to enforce a TTL.
#[derive(Serialize, serde::Deserialize)]
struct Envelope<T> {
    /// Unix timestamp (seconds) when the value was written.
    stored_at: i64,
    value: T,
}

fn state_path(name: &str) -> PathBuf {
    root().join("state").join(format!("{name}.json"))
}

/// Read the cached JSON value `name` only when it was written within `ttl`.
///
/// Returns `None` on a miss, a stale entry, or any read/parse error — callers
/// treat a missing cache as "recompute", so failures degrade to a fresh lookup.
pub fn read_fresh<T: DeserializeOwned>(name: &str, ttl: Duration) -> Option<T> {
    let bytes = fs::read(state_path(name)).ok()?;
    let envelope: Envelope<T> = serde_json::from_slice(&bytes).ok()?;
    let age = chrono::Utc::now()
        .timestamp()
        .saturating_sub(envelope.stored_at);
    if age < 0 || age as u64 > ttl.as_secs() {
        return None;
    }
    Some(envelope.value)
}

/// Write a JSON value under `name`, stamping it with the current time. Best
/// effort: a failure to write the cache is reported but never fatal to callers.
pub fn write<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let path = state_path(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }
    let envelope = Envelope {
        stored_at: chrono::Utc::now().timestamp(),
        value,
    };
    let json = serde_json::to_vec_pretty(&envelope)
        .with_context(|| format!("failed to serialize cache entry {name}"))?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
