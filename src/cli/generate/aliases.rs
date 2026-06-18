use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;

use super::FunctionSignature;
use super::names::alias_names;

/// One `'<alias>' => '<native symbol>'` entry of the alias map.
#[derive(Debug, Serialize)]
struct AliasView {
    alias: String,
    native: String,
}

pub(super) fn render_aliases(signatures: &[FunctionSignature]) -> String {
    let mut aliases = BTreeMap::new();
    for signature in signatures {
        for alias in alias_names(&signature.name) {
            aliases.insert(alias.clone(), signature.name.clone());
            aliases.insert(alias.to_ascii_lowercase(), signature.name.clone());
        }
    }

    // BTreeMap keeps the entries sorted by alias, as the map insertion order did.
    let aliases = aliases
        .into_iter()
        .map(|(alias, native)| AliasView { alias, native })
        .collect::<Vec<_>>();
    super::render_inner_template(
        super::ALIASES_ENTRIES_TEMPLATE,
        json!({ "aliases": aliases }),
    )
}
