use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jsonschema::{Draft, JSONSchema};
use serde_json::{Map, Value};

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

pub fn validate_json_value(kind: SchemaKind, path: &Path, value: &Value) -> Result<()> {
    let version = value
        .get("schema_version")
        .and_then(Value::as_str)
        .with_context(|| format!("{} is missing schema_version", path.display()))?;
    let schema_path = schema_path(kind, version);
    let schema_content = std::fs::read_to_string(&schema_path)
        .with_context(|| format!("failed to read schema {}", schema_path.display()))?;
    let openapi: Value = serde_json::from_str(&schema_content)
        .with_context(|| format!("failed to parse schema {}", schema_path.display()))?;
    let schema = validation_schema_from_openapi(&openapi)
        .with_context(|| format!("failed to load OpenAPI schema {}", schema_path.display()))?;
    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft201909)
        .compile(&schema)
        .map_err(|error| {
            anyhow!(
                "failed to compile schema {}: {error}",
                schema_path.display()
            )
        })?;

    if let Err(errors) = compiled.validate(value) {
        let messages = errors
            .take(5)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "{} does not match {}: {}",
            path.display(),
            schema_path.display(),
            messages
        );
    }

    Ok(())
}

fn schema_path(kind: SchemaKind, version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(kind.directory())
        .join(version)
        .join("schema.json")
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
            if nullable {
                if let Some(Value::String(type_name)) = out.get("type") {
                    out.insert(
                        "type".to_owned(),
                        Value::Array(vec![
                            Value::String(type_name.clone()),
                            Value::String("null".to_owned()),
                        ]),
                    );
                }
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}
