/// Every alias the case-insensitive dispatch map recognizes for a symbol — the
/// original C name plus camelCase and PascalCase spellings.
pub(super) fn alias_names(name: &str) -> Vec<String> {
    let camel = snake_to_camel(name);
    let pascal = snake_to_pascal(name);
    let mut names = vec![name.to_owned(), camel, pascal];
    names.sort();
    names.dedup();
    names
}

/// The spellings emitted as explicit generated methods: the original C name and
/// its camelCase form (no PascalCase — methods read as camelCase). Other casings
/// still resolve dynamically through the alias map.
pub(super) fn method_names(name: &str) -> Vec<String> {
    let mut names = vec![name.to_owned(), snake_to_camel(name)];
    names.sort();
    names.dedup();
    names
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
