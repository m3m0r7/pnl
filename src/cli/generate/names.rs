use super::FunctionSignature;

pub(super) fn alias_names(name: &str) -> Vec<String> {
    let camel = snake_to_camel(name);
    let pascal = snake_to_pascal(name);
    let mut names = vec![name.to_owned(), camel, pascal];
    names.sort();
    names.dedup();
    names
}

pub(super) fn bridge_symbol_name(signature: &FunctionSignature) -> String {
    format!("pnlx_bridge_{}", signature.name)
}

fn snake_to_camel(name: &str) -> String {
    let pascal = snake_to_pascal(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_lowercase(), chars.as_str()),
        None => String::new(),
    }
}

fn snake_to_pascal(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<String>()
}
