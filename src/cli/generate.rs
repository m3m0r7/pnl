use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{Map, Value};

use crate::platform::GeneratedMetadata;

mod aliases;
mod bridge;
mod names;
mod php;
mod types;

use aliases::render_aliases;
use bridge::{render_bridge_cdef, render_bridge_functions};
use php::{render_global_functions, render_methods, runtime_variable_name};
use types::sanitize_php_param_name;

const FFI_TEMPLATE: &str = include_str!("templates/ffi.php.tpl");
const ENTITY_TEMPLATE: &str = include_str!("templates/entity.php.tpl");
const CONTEXT_TEMPLATE: &str = include_str!("templates/context.php.tpl");
const INDEX_TEMPLATE: &str = include_str!("templates/index.php.tpl");
const ALIASES_TEMPLATE: &str = include_str!("templates/aliases.php.tpl");
const FUNCTIONS_TEMPLATE: &str = include_str!("templates/functions.php.tpl");
const BRIDGE_TEMPLATE: &str = include_str!("templates/bridge.rs.tpl");

pub fn generate_ffi_php_from_cdef(cdef: &str, out: &Path) -> Result<()> {
    let mut context = generated_template_context();
    context.insert("CDEF".to_owned(), Value::String(cdef.to_owned()));
    let generated = render_handlebars(FFI_TEMPLATE, context)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

pub fn generate_bridge_ffi_php(signatures: &[FunctionSignature], out: &Path) -> Result<()> {
    generate_ffi_php_from_cdef(&render_bridge_cdef(signatures), out)
}

#[derive(Debug, Clone)]
pub struct PhpPackageTemplateOptions<'a> {
    pub namespace: &'a str,
    pub class_name: &'a str,
    pub library_key: &'a str,
    pub ffi_file: &'a str,
    pub signatures: &'a [FunctionSignature],
    /// Optional extra class name to expose via `class_alias`.
    pub alias_class: Option<&'a str>,
    /// Prefix prepended to every generated method/function name ("" = none).
    pub function_prefix: &'a str,
}

pub fn generate_entity_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, ENTITY_TEMPLATE, options)
}

pub fn generate_context_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, CONTEXT_TEMPLATE, options)
}

pub fn generate_index_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, INDEX_TEMPLATE, options)
}

pub fn generate_aliases_php(out: &Path, signatures: &[FunctionSignature]) -> Result<()> {
    let mut context = generated_template_context();
    context.insert(
        "ALIASES".to_owned(),
        Value::String(render_aliases(signatures)),
    );
    let generated = render_handlebars(ALIASES_TEMPLATE, context)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

pub fn generate_functions_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    let mut context = generated_template_context();
    // Built in Rust so the backslashes never sit next to a `{{ }}` placeholder
    // (Handlebars treats `\{{` as an escape).
    context.insert(
        "FUNC_NAMESPACE".to_owned(),
        Value::String(format!("Pnlx\\Func\\{}", options.class_name)),
    );
    context.insert(
        "FUNCTIONS".to_owned(),
        Value::String(render_global_functions(options)),
    );
    let generated = render_handlebars(FUNCTIONS_TEMPLATE, context)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

pub fn generate_bridge_rs(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    signatures: &[FunctionSignature],
) -> Result<()> {
    let mut context = generated_template_context();
    context.insert(
        "CLASS".to_owned(),
        Value::String(options.class_name.to_owned()),
    );
    context.insert(
        "FUNCTIONS".to_owned(),
        Value::String(render_bridge_functions(signatures)),
    );
    let generated = render_handlebars(BRIDGE_TEMPLATE, context)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

fn write_template(
    out: &Path,
    template: &str,
    options: &PhpPackageTemplateOptions<'_>,
) -> Result<()> {
    let generated = render_template(template, options)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

fn render_template(template: &str, options: &PhpPackageTemplateOptions<'_>) -> Result<String> {
    let mut context = generated_template_context();
    context.insert(
        "NAMESPACE".to_owned(),
        Value::String(options.namespace.to_owned()),
    );
    context.insert(
        "CLASS".to_owned(),
        Value::String(options.class_name.to_owned()),
    );
    context.insert(
        "FQCN".to_owned(),
        Value::String(format!("\\{}\\{}", options.namespace, options.class_name)),
    );
    context.insert(
        "LIBRARY_KEY".to_owned(),
        Value::String(options.library_key.to_owned()),
    );
    context.insert(
        "FFI_FILE".to_owned(),
        Value::String(options.ffi_file.to_owned()),
    );
    context.insert(
        "RUNTIME_VAR".to_owned(),
        Value::String(runtime_variable_name(options)),
    );
    context.insert("METHODS".to_owned(), Value::String(render_methods(options)));
    // `--alias-class` exposes the generated class under an additional name while
    // keeping the original. Emitted in index.php; empty for other templates.
    let class_alias = match options.alias_class {
        Some(alias) if !alias.is_empty() => format!(
            "class_alias(\\{}\\{}::class, {});\n",
            options.namespace,
            options.class_name,
            php_class_literal(alias),
        ),
        _ => String::new(),
    };
    context.insert("CLASS_ALIAS".to_owned(), Value::String(class_alias));
    render_handlebars(template, context)
}

/// Render a class name as a PHP `::class`-style string literal, e.g. `\Foo\Bar::class`.
fn php_class_literal(class: &str) -> String {
    let normalized = class.trim_start_matches('\\');
    format!("\\{normalized}::class")
}

fn generated_template_context() -> Map<String, Value> {
    let metadata = GeneratedMetadata::current();
    let mut context = Map::new();
    context.insert(
        "GENERATED_AT".to_owned(),
        Value::String(metadata.generated_at),
    );
    context.insert("GENERATED_HOST".to_owned(), Value::String(metadata.host));
    context.insert("GENERATED_OS".to_owned(), Value::String(metadata.os));
    context.insert(
        "GENERATED_PHP_VERSION".to_owned(),
        Value::String(metadata.php_version),
    );
    context
}

fn render_handlebars(template: &str, context: Map<String, Value>) -> Result<String> {
    let mut handlebars = Handlebars::new();
    handlebars.register_escape_fn(handlebars::no_escape);
    handlebars
        .render_template(template, &context)
        .context("failed to render generated template")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    pub name: String,
    pub(super) return_type: String,
    pub(super) params: Vec<FunctionParam>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FunctionParam {
    pub(super) name: String,
    pub(super) type_name: String,
}

pub fn parse_function_signatures(cdef: &str) -> Vec<FunctionSignature> {
    let mut seen = BTreeSet::new();
    cdef.lines()
        .filter_map(parse_function_signature)
        .filter(|signature| seen.insert(signature.name.clone()))
        .collect()
}

fn parse_function_signature(line: &str) -> Option<FunctionSignature> {
    let line = line.trim();
    if !line.ends_with(';')
        || !line.contains('(')
        || line.contains("(*")
        || line.starts_with("typedef ")
        // Struct/enum/union definitions and aggregates carry braces; never a
        // plain function prototype.
        || line.contains('{')
        || line.contains('}')
        || line.starts_with("struct ")
        || line.starts_with("union ")
        || line.starts_with("enum ")
    {
        return None;
    }

    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let before = line[..open].trim();
    let (return_type, name) = split_c_declaration_name(before)?;
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }

    Some(FunctionSignature {
        name,
        return_type,
        params: parse_params(line[open + 1..close].trim()),
    })
}

fn split_c_declaration_name(declaration: &str) -> Option<(String, String)> {
    let declaration = declaration.trim();
    let end = declaration
        .char_indices()
        .rev()
        .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index + ch.len_utf8()))?;
    let trimmed = &declaration[..end];
    let mut start = trimmed.len();
    for (index, ch) in trimmed.char_indices().rev() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            start = index;
        } else {
            break;
        }
    }
    if start == trimmed.len() {
        return None;
    }

    let name = trimmed[start..].to_owned();
    let type_name = trimmed[..start].trim().to_owned();
    if type_name.is_empty() {
        None
    } else {
        Some((type_name, name))
    }
}

fn parse_params(params: &str) -> Vec<FunctionParam> {
    if params.trim().is_empty() || params.trim() == "void" {
        return Vec::new();
    }

    let mut seen = BTreeMap::new();
    params
        .split(',')
        .enumerate()
        .map(|(index, param)| unique_param_name(parse_param(param, index), &mut seen))
        .collect()
}

fn unique_param_name(
    mut param: FunctionParam,
    seen: &mut BTreeMap<String, usize>,
) -> FunctionParam {
    let count = seen.entry(param.name.clone()).or_default();
    if *count > 0 {
        param.name = format!("{}_{}", param.name, count);
    }
    *count += 1;

    param
}

fn parse_param(param: &str, index: usize) -> FunctionParam {
    let param = param.trim();
    if let Some((type_name, name)) = split_c_declaration_name(param) {
        return FunctionParam {
            name: sanitize_php_param_name(&name, index),
            type_name,
        };
    }

    FunctionParam {
        name: format!("arg{index}"),
        type_name: param.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CDEF: &str = "const char *demo_name(void);\n\
int demo_add(int left, int right);\n\
void demo_log(const char *message);\n\
double demo_scale(double value, int factor);\n";

    fn sample_signatures() -> Vec<FunctionSignature> {
        parse_function_signatures(SAMPLE_CDEF)
    }

    fn sample_options(signatures: &[FunctionSignature]) -> PhpPackageTemplateOptions<'_> {
        PhpPackageTemplateOptions {
            namespace: "Demo\\Native",
            class_name: "Demo",
            library_key: "demo",
            ffi_file: "demo.ffi.php",
            signatures,
            alias_class: None,
            function_prefix: "",
        }
    }

    #[test]
    fn renders_php_methods() {
        let signatures = sample_signatures();
        insta::assert_snapshot!(render_methods(&sample_options(&signatures)));
    }

    #[test]
    fn renders_php_global_functions() {
        let signatures = sample_signatures();
        insta::assert_snapshot!(render_global_functions(&sample_options(&signatures)));
    }

    #[test]
    fn renders_php_aliases() {
        insta::assert_snapshot!(render_aliases(&sample_signatures()));
    }

    #[test]
    fn renders_bridge_cdef() {
        insta::assert_snapshot!(render_bridge_cdef(&sample_signatures()));
    }

    #[test]
    fn renders_bridge_functions() {
        insta::assert_snapshot!(render_bridge_functions(&sample_signatures()));
    }

    #[test]
    fn bridge_escapes_rust_keyword_parameters() {
        // `fn` is a Rust keyword; the bridge must rename it (SDL_CreateThread has
        // a parameter literally named `fn`).
        let signatures = parse_function_signatures("void demo_thread(int fn, void *data);\n");
        let bridge = render_bridge_functions(&signatures);

        assert!(bridge.contains("fn_: c_int"), "{bridge}");
        assert!(!bridge.contains("(fn:"), "{bridge}");
        assert!(
            bridge.contains("native::demo_thread(fn_, data)"),
            "{bridge}"
        );
    }

    #[test]
    fn struct_definitions_are_not_parsed_as_functions() {
        // A one-line struct definition contains `(` (e.g. inside a field type)
        // but must never be read as a function prototype.
        let cdef = "struct ex_bind { int kind; union { int a; int b; } value; };\n\
int ex_real(int x);\n";
        let signatures = parse_function_signatures(cdef);
        let names: Vec<&str> = signatures.iter().map(|sig| sig.name.as_str()).collect();
        assert_eq!(names, vec!["ex_real"]);
    }
}
