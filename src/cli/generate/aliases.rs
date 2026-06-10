use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;

use super::FunctionSignature;
use super::names::{alias_names, bridge_symbol_name};

/// One `'<alias>' => '<bridge symbol>'` entry of the alias map.
#[derive(Debug, Serialize)]
struct AliasView {
    alias: String,
    bridge: String,
}

pub(super) fn render_aliases(signatures: &[FunctionSignature]) -> String {
    let mut aliases = BTreeMap::new();
    for signature in signatures {
        let bridge = bridge_symbol_name(signature);
        for alias in alias_names(&signature.name) {
            aliases.insert(alias.clone(), bridge.clone());
            aliases.insert(alias.to_ascii_lowercase(), bridge.clone());
        }
    }

    // BTreeMap keeps the entries sorted by alias, as the map insertion order did.
    let aliases = aliases
        .into_iter()
        .map(|(alias, bridge)| AliasView { alias, bridge })
        .collect::<Vec<_>>();
    super::render_inner_template(
        super::ALIASES_ENTRIES_TEMPLATE,
        json!({ "aliases": aliases }),
    )
}
