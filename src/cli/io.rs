use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::{SchemaKind, validate_json_value};

pub fn read_json<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse JSON {}", path.display()))?;
    if let Some(kind) = SchemaKind::from_path(path) {
        validate_json_value(kind, path, &value)?;
    }
    serde_json::from_value(value)
        .with_context(|| format!("failed to parse JSON {}", path.display()))
}

pub fn read_or_default<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if path.exists() {
        read_json(path)
    } else {
        Ok(T::default())
    }
}

pub fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    atomic_write(path, format!("{content}\n").as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("json");
    let temp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));

    {
        let mut file = fs::File::create(&temp)
            .with_context(|| format!("failed to create {}", temp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write {}", temp.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp.display()))?;
    }

    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(_error) if cfg!(windows) && path.exists() => {
            fs::remove_file(path)
                .with_context(|| format!("failed to replace {}", path.display()))?;
            fs::rename(&temp, path).with_context(|| format!("failed to write {}", path.display()))
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error).with_context(|| format!("failed to write {}", path.display()))
        }
    }
}

pub fn write_json_if_missing<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if path.exists() {
        crate::ui::info(&format!("{} already exists", path.display()));
        return Ok(());
    }
    write_json(path, value)
}
