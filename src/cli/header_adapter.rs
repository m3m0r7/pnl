mod support;

use support::{
    NormalizationMode, brace_delta, collect_referenced_structs, extract_struct_definitions,
    normalize_c_fragment, remove_abi_macros, replace_enum_types, strip_leading_attribute_macros,
    strip_trailing_attribute_macros,
};

#[derive(Debug, Clone)]
pub struct HeaderAdapterOptions {
    pub symbol_prefix: String,
}

pub fn cdef_from_header(header: &str, options: &HeaderAdapterOptions) -> String {
    let prefix = options.symbol_prefix.trim();
    if prefix.is_empty() {
        return header.to_owned();
    }

    let stripped = strip_c_comments(header);
    let typedefs = extract_typedefs(&stripped, prefix);
    let enum_typedefs = extract_enum_typedefs(&stripped, prefix);
    let functions = extract_functions(&stripped, prefix);
    let structs = extract_struct_definitions(&stripped, prefix);
    let referenced_structs = collect_referenced_structs(&typedefs, &functions);

    let mut out = String::new();
    out.push_str("typedef signed long ssize_t;\n");
    out.push_str("typedef unsigned long size_t;\n");
    out.push_str("typedef long intptr_t;\n");
    out.push_str("typedef unsigned char uint8_t;\n");
    out.push_str("typedef unsigned short uint16_t;\n");
    out.push_str("typedef unsigned int uint32_t;\n");
    out.push_str("typedef unsigned long long uint64_t;\n\n");
    out.push_str("typedef unsigned char Uint8;\n");
    out.push_str("typedef unsigned short Uint16;\n");
    out.push_str("typedef unsigned int Uint32;\n\n");

    out.push_str("struct timeval;\n");
    for name in referenced_structs.keys() {
        if name == "timeval" || structs.contains_key(name) {
            continue;
        }
        out.push_str("struct ");
        out.push_str(name);
        out.push_str(";\n");
    }
    for name in structs.keys() {
        out.push_str("struct ");
        out.push_str(name);
        out.push_str(";\n");
    }

    out.push('\n');
    for name in structs.keys() {
        out.push_str("typedef struct ");
        out.push_str(name);
        out.push(' ');
        out.push_str(name);
        out.push_str(";\n");
    }
    for typedef in typedefs
        .iter()
        .filter(|typedef| typedef.starts_with("typedef struct "))
        .chain(enum_typedefs.iter())
        .chain(
            typedefs
                .iter()
                .filter(|typedef| !typedef.starts_with("typedef struct ")),
        )
    {
        out.push_str(typedef);
        out.push('\n');
    }

    out.push('\n');
    out.push_str("struct timeval { long tv_sec; int tv_usec; };\n");
    for declaration in structs.values() {
        out.push_str(declaration);
        out.push('\n');
    }

    out.push('\n');
    for function in functions {
        out.push_str(&function);
        out.push('\n');
    }

    out
}

fn strip_c_comments(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(ch) = chars.next() {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for ch in chars.by_ref() {
                if ch == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn extract_typedefs(input: &str, prefix: &str) -> Vec<String> {
    extract_declarations(input, prefix, DeclarationKind::Typedef)
}

fn extract_enum_typedefs(input: &str, prefix: &str) -> Vec<String> {
    let mut declarations = Vec::new();
    let mut current = String::new();

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if current.is_empty() && !line.starts_with("typedef enum") {
            continue;
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line);

        if line.ends_with(';') {
            if let Some(name) = enum_typedef_name(&current, prefix) {
                declarations.push(format!("typedef int {name};"));
            }
            current.clear();
        }
    }

    declarations.sort();
    declarations.dedup();
    declarations
}

fn enum_typedef_name(declaration: &str, prefix: &str) -> Option<String> {
    let close = declaration.rfind('}')?;
    declaration[close + 1..]
        .trim()
        .trim_end_matches(';')
        .split(',')
        .next_back()
        .map(str::trim)
        .filter(|name| name.starts_with(prefix))
        .map(ToOwned::to_owned)
}

fn extract_functions(input: &str, prefix: &str) -> Vec<String> {
    extract_declarations(input, prefix, DeclarationKind::Function)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeclarationKind {
    Typedef,
    Function,
}

fn extract_declarations(input: &str, prefix: &str, kind: DeclarationKind) -> Vec<String> {
    let mut declarations = Vec::new();
    let mut current = String::new();
    let mut skip_inline_depth: Option<i32> = None;
    let mut skip_preprocessor_continuation = false;

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if skip_preprocessor_continuation {
            skip_preprocessor_continuation = line.ends_with('\\');
            continue;
        }
        if let Some(depth) = skip_inline_depth.as_mut() {
            *depth += brace_delta(line);
            if *depth <= 0 && line.contains('}') {
                skip_inline_depth = None;
            }
            continue;
        }

        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            skip_preprocessor_continuation = line.ends_with('\\');
            continue;
        }

        if line.starts_with("static inline") {
            let depth = brace_delta(line);
            if depth > 0 {
                skip_inline_depth = Some(depth);
            } else if !line.ends_with(';') {
                skip_inline_depth = Some(0);
            }
            continue;
        }

        if current.is_empty() && !starts_candidate_declaration(line, prefix, kind) {
            continue;
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line);

        if line.ends_with(';') {
            if let Some(declaration) = normalize_declaration(&current, prefix, kind) {
                declarations.push(declaration);
            }
            current.clear();
        }
    }

    declarations.sort();
    declarations.dedup();
    declarations
}

fn starts_candidate_declaration(line: &str, prefix: &str, kind: DeclarationKind) -> bool {
    match kind {
        DeclarationKind::Typedef => {
            line.starts_with("typedef ")
                && line.contains(prefix)
                && (!line.starts_with("typedef struct ")
                    || line.ends_with(';') && !line.contains('{'))
        }
        DeclarationKind::Function => {
            !line.starts_with("typedef ")
                && line.contains(prefix)
                && line.contains('(')
                && !line.contains('=')
        }
    }
}

fn normalize_declaration(declaration: &str, prefix: &str, kind: DeclarationKind) -> Option<String> {
    if declaration.contains("...")
        || declaration.contains('{') && declaration.starts_with("typedef enum ")
    {
        return None;
    }

    let mut out = strip_leading_attribute_macros(declaration);
    out = remove_abi_macros(&out);
    out = strip_trailing_attribute_macros(&out);
    out = replace_enum_types(&out, prefix);
    out = normalize_c_fragment(&out, NormalizationMode::Declaration);

    if kind == DeclarationKind::Function && !valid_function_declaration(&out, prefix) {
        return None;
    }

    if out.contains(prefix) {
        Some(out)
    } else {
        None
    }
}

fn valid_function_declaration(declaration: &str, prefix: &str) -> bool {
    if declaration.contains("(*") {
        return false;
    }

    let Some(open) = declaration.find('(') else {
        return false;
    };
    let before = declaration[..open].trim();
    let Some(name) = before
        .rsplit(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .find(|part| !part.is_empty())
    else {
        return false;
    };

    name.starts_with(prefix) && !is_macro_like_function_name(name, prefix)
}

fn is_macro_like_function_name(name: &str, prefix: &str) -> bool {
    let rest = name.trim_start_matches(prefix).trim_start_matches('_');
    !rest.is_empty()
        && rest
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}
