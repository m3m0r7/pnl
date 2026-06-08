use std::collections::BTreeMap;

use super::FunctionSignature;
use super::names::{alias_names, bridge_symbol_name};

pub(super) fn render_aliases(signatures: &[FunctionSignature]) -> String {
    let mut aliases = BTreeMap::new();
    for signature in signatures {
        let bridge = bridge_symbol_name(signature);
        for alias in alias_names(&signature.name) {
            aliases.insert(alias.clone(), bridge.clone());
            aliases.insert(alias.to_ascii_lowercase(), bridge.clone());
        }
    }

    let mut lines = String::new();
    for (alias, native) in aliases {
        lines.push_str("    '");
        lines.push_str(&alias);
        lines.push_str("' => '");
        lines.push_str(&native);
        lines.push_str("',\n");
    }
    lines
}
