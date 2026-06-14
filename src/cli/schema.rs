use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use include_dir::{Dir, include_dir};
use jsonschema::{Draft, JSONSchema};
use serde_json::{Map, Value};

/// The JSON schemas are embedded into the binary at build time so validation
/// works no matter where the executable is run from.
static SCHEMAS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/schemas");

#[derive(Debug, Clone, Copy)]
pub enum SchemaKind {
    Pnl,
    Pnlx,
    PnlxLock,
    PnlxPathmap,
    RepositoryIndex,
}

impl SchemaKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.file_name().and_then(|value| value.to_str()) {
            Some("pnl.json") => Some(Self::Pnl),
            Some("pnlx.json") => Some(Self::Pnlx),
            Some("pnlx-lock.json") => Some(Self::PnlxLock),
            Some("pnlx-pathmap.json") => Some(Self::PnlxPathmap),
            Some("repository-index.json") => Some(Self::RepositoryIndex),
            _ => None,
        }
    }

    /// Resolve a schema by its short name (`pnl`, `pnlx`, `pnlx-lock`,
    /// `pnlx-pathmap`, `repository-index`) — the form the PHP runtime passes over FFI.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pnl" => Some(Self::Pnl),
            "pnlx" => Some(Self::Pnlx),
            "pnlx-lock" => Some(Self::PnlxLock),
            "pnlx-pathmap" => Some(Self::PnlxPathmap),
            "repository-index" => Some(Self::RepositoryIndex),
            _ => None,
        }
    }

    fn directory(self) -> &'static str {
        match self {
            Self::Pnl => "pnl",
            Self::Pnlx => "pnlx",
            Self::PnlxLock => "pnlx-lock",
            Self::PnlxPathmap => "pnlx-pathmap",
            Self::RepositoryIndex => "repository-index",
        }
    }
}

/// Validate a JSON string against the schema for `kind`. Used by the FFI bridge
/// so the PHP runtime can re-validate workspace files without an OpenAPI library.
pub fn validate_json_str(kind: SchemaKind, json: &str) -> Result<()> {
    let value: Value = serde_json::from_str(json).context("input is not valid JSON")?;
    validate_json_value(kind, Path::new("<input>"), &value)
}

pub fn validate_json_value(kind: SchemaKind, path: &Path, value: &Value) -> Result<()> {
    let version = value
        .get("schema_version")
        .and_then(Value::as_str)
        .with_context(|| format!("{} is missing schema_version", path.display()))?;
    let schema_ref = format!("{}/{version}/schema.json", kind.directory());
    let schema_content = embedded_schema(&schema_ref)?;
    let openapi: Value = serde_json::from_str(schema_content)
        .with_context(|| format!("failed to parse schema {schema_ref}"))?;
    let schema = validation_schema_from_openapi(&openapi)
        .with_context(|| format!("failed to load OpenAPI schema {schema_ref}"))?;
    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft201909)
        .compile(&schema)
        .map_err(|error| anyhow!("failed to compile schema {schema_ref}: {error}"))?;

    if let Err(errors) = compiled.validate(value) {
        let messages = errors
            .take(5)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "{} does not match schema {schema_ref}: {messages}",
            path.display(),
        );
    }

    Ok(())
}

/// Look up a schema embedded in the binary by its `<kind>/<version>/schema.json` path.
fn embedded_schema(schema_ref: &str) -> Result<&'static str> {
    SCHEMAS
        .get_file(schema_ref)
        .and_then(|file| file.contents_utf8())
        .with_context(|| format!("unknown or unsupported schema: {schema_ref}"))
}

fn validation_schema_from_openapi(openapi: &Value) -> Result<Value> {
    let schemas = openapi
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .context("OpenAPI document is missing components.schemas")?;
    let document = schemas
        .get("Document")
        .context("OpenAPI document is missing components.schemas.Document")?;

    let mut root = normalize_openapi_schema(document);
    let root_object = root
        .as_object_mut()
        .context("components.schemas.Document must be an object")?;
    root_object.insert(
        "$schema".to_owned(),
        Value::String("https://json-schema.org/draft/2019-09/schema".to_owned()),
    );

    let mut defs = Map::new();
    for (name, schema) in schemas {
        if name == "Document" {
            continue;
        }
        defs.insert(name.clone(), normalize_openapi_schema(schema));
    }
    root_object.insert("$defs".to_owned(), Value::Object(defs));

    Ok(root)
}

fn normalize_openapi_schema(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(normalize_openapi_schema).collect()),
        Value::Object(object) => {
            let mut out = Map::new();
            let nullable = object
                .get("nullable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            for (key, item) in object {
                match key.as_str() {
                    "nullable" => {}
                    "$ref" => {
                        if let Some(reference) = item.as_str() {
                            out.insert(
                                key.clone(),
                                Value::String(
                                    reference.replace("#/components/schemas/", "#/$defs/"),
                                ),
                            );
                        }
                    }
                    "x-propertyNames" => {
                        out.insert("propertyNames".to_owned(), normalize_openapi_schema(item));
                    }
                    _ => {
                        out.insert(key.clone(), normalize_openapi_schema(item));
                    }
                }
            }
            if nullable && let Some(Value::String(type_name)) = out.get("type") {
                out.insert(
                    "type".to_owned(),
                    Value::Array(vec![
                        Value::String(type_name.clone()),
                        Value::String("null".to_owned()),
                    ]),
                );
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{SchemaKind, validate_json_str};
    use crate::SCHEMA_VERSION;

    #[test]
    fn validates_alias_pnlx_manifest() {
        let alias = format!(r#"{{"schema_version": "{SCHEMA_VERSION}", "ref": "../libsdl"}}"#);
        validate_json_str(SchemaKind::Pnlx, &alias).unwrap();
    }

    #[test]
    fn rejects_alias_pnlx_with_extra_package_fields() {
        // An alias manifest is `oneOf` the alias or full form; mixing a `ref`
        // with package-only fields like `version` must not validate as either.
        let bad = format!(
            r#"{{"schema_version": "{SCHEMA_VERSION}", "ref": "../libsdl", "version": "1.0.0"}}"#
        );
        assert!(validate_json_str(SchemaKind::Pnlx, &bad).is_err());
    }

    #[test]
    fn validates_repository_index_alias_entry() {
        let index = format!(
            r#"{{"schema_version": "{SCHEMA_VERSION}", "packages": {{"sdl": {{"ref": "libsdl"}}}}}}"#
        );
        validate_json_str(SchemaKind::RepositoryIndex, &index).unwrap();
    }
}
