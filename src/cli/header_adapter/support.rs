use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NormalizationMode {
    Declaration,
    Struct,
}

pub(super) fn strip_leading_attribute_macros(declaration: &str) -> String {
    let mut out = declaration.trim().to_owned();
    loop {
        let Some(open) = out.find('(') else {
            return out;
        };
        let token = out[..open].trim();
        if token.is_empty()
            || !token
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
        {
            return out;
        }
        let Some(close) = matching_paren(out.as_bytes(), open) else {
            return out;
        };
        out = out[close + 1..].trim_start().to_owned();
    }
}

pub(super) fn strip_trailing_attribute_macros(declaration: &str) -> String {
    let mut out = declaration.trim().to_owned();
    loop {
        let trimmed = out.trim_end_matches(';').trim_end();
        let Some(open) = trimmed.rfind('(') else {
            return out;
        };
        let token_start = trimmed[..open]
            .char_indices()
            .rev()
            .find_map(|(index, ch)| {
                (!ch.is_ascii_alphanumeric() && ch != '_').then_some(index + ch.len_utf8())
            })
            .unwrap_or(0);
        let token = trimmed[token_start..open].trim();
        if token.is_empty()
            || !token
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
        {
            return out;
        }
        let Some(close) = matching_paren(trimmed.as_bytes(), open) else {
            return out;
        };
        if close + 1 != trimmed.len() {
            return out;
        }
        out = format!("{};", trimmed[..token_start].trim_end());
    }
}

pub(super) fn remove_abi_macros(declaration: &str) -> String {
    declaration
        .split_whitespace()
        .filter_map(|token| {
            let bare = token.trim_matches(|ch: char| {
                ch == '(' || ch == ')' || ch == ';' || ch == ',' || ch == '*'
            });
            if bare == "extern"
                || (bare.contains('_')
                    && bare
                        .chars()
                        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit()))
                || (bare.contains("CALL")
                    && bare
                        .chars()
                        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit()))
                || matches!(bare, "DECLSPEC")
            {
                let kept = token.replace(bare, "");
                (!kept.is_empty()).then_some(kept)
            } else {
                Some(token.to_owned())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn replace_enum_types(input: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(index) = rest.find("enum ") {
        out.push_str(&rest[..index]);
        let after = &rest[index + "enum ".len()..];
        let name = after
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect::<String>();
        if name.starts_with(prefix) {
            out.push_str("int");
            rest = &after[name.len()..];
        } else {
            out.push_str("enum ");
            out.push_str(&name);
            rest = &after[name.len()..];
        }
    }
    out.push_str(rest);
    out
}

pub(super) fn extract_struct_definitions(input: &str, prefix: &str) -> BTreeMap<String, String> {
    let mut structs = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut offset = 0;

    while let Some(relative) = input[offset..].find("struct ") {
        let start = offset + relative;
        let name_start = start + "struct ".len();
        let name = input[name_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect::<String>();
        if name.is_empty() {
            offset = name_start;
            continue;
        }

        let after_name = name_start + name.len();
        let rest = &input[after_name..];
        let trimmed = rest.trim_start();
        if !name.starts_with(prefix) || !trimmed.starts_with('{') {
            offset = after_name;
            continue;
        }

        let open = after_name + rest.len() - trimmed.len();
        let Some(close) = matching_brace(bytes, open) else {
            offset = after_name;
            continue;
        };
        let after_close = &input[close + 1..];
        let Some(semi_relative) = after_close.find(';') else {
            offset = close + 1;
            continue;
        };
        let end = close + 1 + semi_relative + 1;
        let declaration = &input[start..end];

        if let Some(normalized) = normalize_struct_definition(declaration, prefix) {
            structs.insert(name, normalized);
        }

        offset = end;
    }

    structs
}

pub(super) fn normalize_c_fragment(input: &str, mode: NormalizationMode) -> String {
    let mut out = input
        .replace("__attribute__((packed))", "")
        .replace("__attribute__ ((packed))", "")
        .replace("LIBUSB_PACKED", "")
        .replace("[0]", "[]")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    for (from, to) in [
        ("( *", "(*"),
        (" * ", " *"),
        (" ** ", " **"),
        (" []", "[]"),
        (" ;", ";"),
    ] {
        out = out.replace(from, to);
    }

    if mode == NormalizationMode::Struct {
        out = out.replace(" { ", " { ").replace(" } ", " } ");
    }

    out
}

pub(super) fn collect_referenced_structs(
    typedefs: &[String],
    functions: &[String],
) -> BTreeMap<String, ()> {
    let mut structs = BTreeMap::new();
    for declaration in typedefs.iter().chain(functions.iter()) {
        let mut rest = declaration.as_str();
        while let Some(index) = rest.find("struct ") {
            rest = &rest[index + "struct ".len()..];
            let name = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect::<String>();
            if !name.is_empty() {
                structs.insert(name, ());
            }
        }
    }
    structs
}

pub(super) fn brace_delta(line: &str) -> i32 {
    let opens = line.chars().filter(|ch| *ch == '{').count() as i32;
    let closes = line.chars().filter(|ch| *ch == '}').count() as i32;
    opens - closes
}

fn normalize_struct_definition(declaration: &str, prefix: &str) -> Option<String> {
    if declaration.contains("LIBUSB_FLEXIBLE_ARRAY")
        || declaration.contains("FLEXIBLE_ARRAY")
        || declaration.contains(':')
    {
        return None;
    }

    let mut out = remove_abi_macros(declaration);
    out = replace_enum_types(&out, prefix);
    out = out
        .replace("const uint8_t", "uint8_t")
        .replace("const uint16_t", "uint16_t")
        .replace("const uint32_t", "uint32_t");
    out = normalize_c_fragment(&out, NormalizationMode::Struct);
    out = strip_trailing_struct_alias(&out);

    Some(out)
}

fn strip_trailing_struct_alias(declaration: &str) -> String {
    let Some(close) = declaration.rfind('}') else {
        return declaration.to_owned();
    };
    if declaration[close + 1..]
        .trim()
        .trim_end_matches(';')
        .is_empty()
    {
        return declaration.to_owned();
    }

    format!("{};", declaration[..=close].trim_end())
}

fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}
