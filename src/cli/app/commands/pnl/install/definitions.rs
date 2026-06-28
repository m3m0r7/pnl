//! Resolving a package's `compile_options.definitions` (`require_definitions`)
//! to concrete `-D` values at install time, prompting or using lock/default.

use anyhow::Result;

use super::*;

/// Resolve a package's `require_definitions` to concrete values. Resolution order
/// (per the lock-as-source-of-truth model): a value already recorded in the lock
/// (`prior`) preseeds the prompt and is the value a non-interactive install uses;
/// otherwise the declared `default`. An interactive install always prompts
/// (preseeded). A non-interactive install with neither a prior value nor a default
/// errors instead of guessing.
pub(super) fn resolve_require_definitions(
    package: &str,
    definitions: &[RequireDefinition],
    prior: &BTreeMap<String, String>,
    interaction: &crate::app::interaction::Interaction,
) -> Result<Vec<ResolvedDefinition>> {
    let mut resolved = Vec::new();
    for definition in definitions {
        let initial = prior
            .get(&definition.name)
            .cloned()
            .or_else(|| definition.default.as_ref().map(definition_default_string));
        let value = if interaction.can_prompt() {
            prompt_definition(definition, initial.as_deref(), interaction)?
        } else {
            let value = initial.clone().ok_or_else(|| {
                anyhow!(
                    "{package} requires the build-time definition `{name}`, but no value \
                     is available (no default, none recorded in pnlx-lock.json, and this \
                     is a non-interactive install). Run install interactively or record a \
                     value in the lockfile.",
                    name = definition.name,
                )
            })?;
            validate_definition_value(definition.definition_type, &value)
                .map_err(|reason| anyhow!("invalid value for `{}`: {reason}", definition.name))?;
            value
        };
        resolved.push(ResolvedDefinition {
            name: definition.name.clone(),
            value,
            definition_type: definition.definition_type,
        });
    }
    Ok(resolved)
}

/// Prompt for one definition's value, re-asking until it validates. A `boolean`
/// uses the Y/n selector; the other types read a typed line (empty keeps the
/// preseeded `initial`).
fn prompt_definition(
    definition: &RequireDefinition,
    initial: Option<&str>,
    interaction: &crate::app::interaction::Interaction,
) -> Result<String> {
    if definition.definition_type == DefinitionType::Boolean {
        let question = if definition.description.is_empty() {
            definition.name.clone()
        } else {
            format!("{} — {}", definition.name, definition.description)
        };
        let yes = interaction.confirm(&question, matches!(initial, Some("1")))?;
        return Ok(if yes { "1" } else { "0" }.to_owned());
    }
    loop {
        let raw = interaction.read_value(&definition.name, &definition.description, initial)?;
        let candidate = if raw.is_empty() {
            initial.map(str::to_owned)
        } else {
            Some(normalize_definition_value(definition.definition_type, &raw))
        };
        let Some(value) = candidate else {
            crate::app::ui::warn("a value is required");
            continue;
        };
        match validate_definition_value(definition.definition_type, &value) {
            Ok(()) => return Ok(value),
            Err(reason) => crate::app::ui::warn(&reason),
        }
    }
}

/// A declared JSON default rendered as the string the solver carries (a boolean as
/// `1`/`0`, a string verbatim, a number as its text).
fn definition_default_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(flag) => if *flag { "1" } else { "0" }.to_owned(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Normalise raw input to the canonical stored form (a boolean to `1`/`0`).
fn normalize_definition_value(definition_type: DefinitionType, raw: &str) -> String {
    if definition_type != DefinitionType::Boolean {
        return raw.trim().to_owned();
    }
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "y" | "yes" | "true" | "t" | "on" => "1".to_owned(),
        "0" | "n" | "no" | "false" | "f" | "off" => "0".to_owned(),
        other => other.to_owned(),
    }
}

/// Validate a solved value against its declared type.
fn validate_definition_value(
    definition_type: DefinitionType,
    value: &str,
) -> std::result::Result<(), String> {
    match definition_type {
        DefinitionType::Int => value
            .parse::<i128>()
            .map(|_| ())
            .map_err(|_| format!("`{value}` is not an integer")),
        DefinitionType::Float => value
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("`{value}` is not a number")),
        DefinitionType::String => Ok(()),
        DefinitionType::Boolean => (value == "0" || value == "1")
            .then_some(())
            .ok_or_else(|| format!("`{value}` is not a boolean (enter y/n)")),
    }
}
