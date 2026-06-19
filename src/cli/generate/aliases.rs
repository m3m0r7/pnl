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
        // An unsupported (e.g. `static inline`) function has no real export to
        // dispatch to — its method throws — so it gets no alias-map entry.
        if signature.unsupported.is_some() {
            continue;
        }
        // The map resolves a public/ergonomic name to the exported C symbol, which
        // differs from the public name for symbol-version renames (ICU).
        let native = signature.native_symbol();
        for alias in alias_names(&signature.name) {
            aliases.insert(alias.clone(), native.to_owned());
            aliases.insert(alias.to_ascii_lowercase(), native.to_owned());
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
