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
use php::{render_global_functions, render_methods};
use types::sanitize_php_param_name;

const FFI_TEMPLATE: &str = include_str!("templates/package/src/generated/ffi.php.tpl");
const ENTITY_TEMPLATE: &str = include_str!("templates/package/src/generated/entity.php.tpl");
const MANIFEST_TEMPLATE: &str = include_str!("templates/package/src/generated/manifest.php.tpl");
const CONTEXT_TEMPLATE: &str = include_str!("templates/package/src/generated/context.php.tpl");
const EXCEPTION_TEMPLATE: &str = include_str!("templates/package/src/generated/exception.php.tpl");
const TYPE_FILE_TEMPLATE: &str = include_str!("templates/package/src/generated/types.php.tpl");
const CONST_TEMPLATE: &str = include_str!("templates/package/src/generated/const.php.tpl");
const MACRO_FUNCTIONS_TEMPLATE: &str =
    include_str!("templates/package/src/generated/macro.functions.php.tpl");
const AUTOLOAD_TEMPLATE: &str = include_str!("templates/workspace/autoload.php.tpl");
const IDE_HELPER_TEMPLATE: &str = include_str!("templates/workspace/ide-helper.php.tpl");
const INDEX_TEMPLATE: &str = include_str!("templates/package/src/generated/index.php.tpl");
const ALIASES_TEMPLATE: &str =
    include_str!("templates/package/src/generated/function.aliases.php.tpl");
const FUNCTIONS_TEMPLATE: &str = include_str!("templates/package/src/generated/functions.php.tpl");
const BRIDGE_TEMPLATE: &str = include_str!("templates/package/src/generated/bridge.rs.tpl");

// Inner templates: the repeated, per-symbol bodies that used to be assembled
// with `format!`/`push_str` in Rust now live here as Handlebars `{{#each}}`
// loops, so the generated layout is editable without touching code.
const METHODS_TEMPLATE: &str = include_str!("templates/partials/methods.php.tpl");
const GLOBAL_FUNCTIONS_TEMPLATE: &str = include_str!("templates/partials/global_functions.php.tpl");
const ALIASES_ENTRIES_TEMPLATE: &str = include_str!("templates/partials/aliases_entries.php.tpl");
const BRIDGE_CDEF_TEMPLATE: &str = include_str!("templates/partials/bridge_cdef.c.tpl");
const BRIDGE_FUNCTIONS_TEMPLATE: &str = include_str!("templates/partials/bridge_functions.rs.tpl");

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
    /// The native library's package name, emitted as the entity class's
    /// `#[NativeLibraryName(...)]` attribute (e.g. `libsdl/libsdl`).
    pub native_library_name: &'a str,
    /// The native library's package version, emitted as the entity class's
    /// `#[NativeLibraryVersion(...)]` attribute (e.g. `2.32.10`).
    pub native_library_version: &'a str,
    /// The package description, baked into the entity as the `DESCRIPTION` const.
    pub description: &'a str,
}

/// Generate one entity variant. `allow_cdata` selects whether pointer parameters
/// also admit a raw `\FFI\CData` (the `cdata/` variant); `scalars_in_return`
/// selects the `scalar/` variant whose methods return PHP-native scalars.
pub fn generate_entity_php(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
    scalars_in_return: bool,
) -> Result<()> {
    write_template(
        out,
        ENTITY_TEMPLATE,
        options,
        allow_cdata,
        scalars_in_return,
    )
}

/// Generate the `<Class>Manifest` metadata class (manifest/bridge accessors).
pub fn generate_manifest_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, MANIFEST_TEMPLATE, options, false, false)
}

/// Generate the `<Class>Context` wrapper class for FFI values.
pub fn generate_context_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, CONTEXT_TEMPLATE, options, false, false)
}

/// Generate the per-extension `<Class>Exception` base exception class.
pub fn generate_exception_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, EXCEPTION_TEMPLATE, options, false, false)
}

pub fn generate_index_php(out: &Path, options: &PhpPackageTemplateOptions<'_>) -> Result<()> {
    write_template(out, INDEX_TEMPLATE, options, false, false)
}

/// Generate `const.php`: the object-like `#define` constants from the header,
/// emitted as namespaced PHP `const`s (referenceable as `\<Namespace>\<NAME>`).
/// `constants` is `(name, php_value_expression)` pairs in source order.
pub fn generate_const_php(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    constants: &[(String, String)],
) -> Result<()> {
    let mut context = generated_template_context();
    context.insert(
        "NAMESPACE".to_owned(),
        Value::String(options.namespace.to_owned()),
    );
    context.insert(
        "constants".to_owned(),
        Value::Array(
            constants
                .iter()
                .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
                .collect(),
        ),
    );
    write_generated(out, render_handlebars(CONST_TEMPLATE, context)?)
}

/// Generate the per-package pointer wrappers into `dir` (`src/generated/types/`),
/// one class per file. Returns the type class names that were written (so the
/// package's `index.php` can require each one).
pub fn generate_types_php(
    dir: &Path,
    options: &PhpPackageTemplateOptions<'_>,
) -> Result<Vec<String>> {
    let base = format!("\\{}\\{}Context", options.namespace, options.class_name);
    let types = php::collect_pointer_types(options);
    for type_name in &types {
        let mut context = generated_template_context();
        context.insert(
            "NAMESPACE".to_owned(),
            Value::String(options.namespace.to_owned()),
        );
        context.insert("BASE".to_owned(), Value::String(base.clone()));
        context.insert("TYPE".to_owned(), Value::String(type_name.clone()));
        write_generated(
            &dir.join(format!("{type_name}.php")),
            render_handlebars(TYPE_FILE_TEMPLATE, context)?,
        )?;
    }
    Ok(types)
}

/// Generate the workspace `@pnlx/autoload.php`. `packages` are the per-package
/// entrypoint paths (already escaped for a single-quoted PHP literal), relative
/// to the workspace. The lockfile is located at runtime through the pathmap.
pub fn generate_autoload_php(
    out: &Path,
    version: &str,
    packages: &[String],
    manifest_path: &str,
) -> Result<()> {
    let mut context = generated_template_context();
    context.insert("VERSION".to_owned(), Value::String(version.to_owned()));
    context.insert(
        "PACKAGES".to_owned(),
        serde_json::to_value(packages).expect("package paths serialize to JSON"),
    );
    // The absolute pnl.json path, escaped for a single-quoted PHP literal.
    context.insert(
        "PROJECT_MANIFEST".to_owned(),
        Value::String(manifest_path.replace('\\', "\\\\").replace('\'', "\\'")),
    );
    // Surface the built-in config.toml values and the binary's build target as
    // PHP constants (PNL_CONFIG_*/PNLX_CONFIG_*, PNLX_BUILD_OS/ARCH).
    context.insert(
        "CONFIG_SCHEMA_VERSION".to_owned(),
        Value::String(crate::config::SCHEMA_VERSION.to_owned()),
    );
    context.insert(
        "CONFIG_SELF_REPOSITORY".to_owned(),
        Value::String(crate::config::SELF_REPOSITORY.to_owned()),
    );
    context.insert(
        "CONFIG_PACKAGES_REPOSITORY".to_owned(),
        Value::String(crate::config::PACKAGES_REPOSITORY.to_owned()),
    );
    context.insert(
        "CONFIG_OUTPUT_DIR".to_owned(),
        Value::String(crate::config::DEFAULT_OUTPUT_DIR.to_owned()),
    );
    context.insert(
        "CONFIG_TTL_SECONDS".to_owned(),
        Value::Number(crate::config::UPDATE_CHECK_TTL_SECONDS.into()),
    );
    context.insert(
        "CONFIG_OPT_OUT_ENV".to_owned(),
        Value::String(crate::config::UPDATE_CHECK_OPT_OUT_ENV.to_owned()),
    );
    context.insert(
        "CONFIG_CACHE_KEY".to_owned(),
        Value::String(crate::config::UPDATE_CHECK_CACHE_KEY.to_owned()),
    );
    context.insert(
        "CONFIG_BINARIES".to_owned(),
        serde_json::to_value(crate::config::BINARIES).expect("binaries serialize to JSON"),
    );
    context.insert(
        "BUILD_OS".to_owned(),
        Value::String(crate::config::BUILD_OS.to_owned()),
    );
    context.insert(
        "BUILD_ARCH".to_owned(),
        Value::String(crate::config::BUILD_ARCH.to_owned()),
    );
    write_generated(out, render_handlebars(AUTOLOAD_TEMPLATE, context)?)
}

/// Generate the workspace `@pnlx/ide-helper.php` (guarded `\FFI` stubs).
pub fn generate_ide_helper_php(out: &Path) -> Result<()> {
    write_generated(
        out,
        render_handlebars(IDE_HELPER_TEMPLATE, generated_template_context())?,
    )
}

fn write_generated(out: &Path, generated: String) -> Result<()> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
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

/// Generate `macro.functions.php`: function-like C macros surfaced as PHP
/// functions under `\Pnlx\Func\<Class>`. Always loaded.
pub fn generate_macro_functions_php(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    macro_functions: &[crate::header_adapter::MacroFunction],
) -> Result<()> {
    let mut context = generated_template_context();
    context.insert(
        "FUNC_NAMESPACE".to_owned(),
        Value::String(format!("Pnlx\\Func\\{}", options.class_name)),
    );
    context.insert(
        "FUNCTIONS".to_owned(),
        Value::String(render_macro_functions(macro_functions, options.class_name)),
    );
    write_generated(out, render_handlebars(MACRO_FUNCTIONS_TEMPLATE, context)?)
}

/// Render the PHP function definitions for the function-like macros, in Rust so
/// the namespace backslashes never sit next to a `{{ }}` placeholder.
fn render_macro_functions(
    macro_functions: &[crate::header_adapter::MacroFunction],
    class_name: &str,
) -> String {
    let mut out = String::new();
    for function in macro_functions {
        let fqn = format!("Pnlx\\Func\\{class_name}\\{}", function.name);
        let params = function
            .params
            .iter()
            .map(|param| format!("${param}"))
            .collect::<Vec<_>>()
            .join(", ");
        let body = match &function.body {
            Ok(expr) => format!("        return {expr};"),
            Err(symbol) => format!(
                "        throw new \\Pnlx\\Exception\\PHPNativeLibraryException('{} calls the undefined C function {}.');",
                function.name, symbol
            ),
        };
        out.push_str(&format!(
            "if (!function_exists('{fqn}')) {{\n    function {}({params})\n    {{\n{body}\n    }}\n}}\n\n",
            function.name
        ));
    }
    out
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
    allow_cdata: bool,
    scalars_in_return: bool,
) -> Result<()> {
    let generated = render_template(template, options, allow_cdata, scalars_in_return)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::ui::created("generated", out);
    Ok(())
}

fn render_template(
    template: &str,
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
    scalars_in_return: bool,
) -> Result<String> {
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
        "LIBRARY_KEY".to_owned(),
        Value::String(options.library_key.to_owned()),
    );
    context.insert(
        "FFI_FILE".to_owned(),
        Value::String(options.ffi_file.to_owned()),
    );
    context.insert(
        "METHODS".to_owned(),
        Value::String(render_methods(options, allow_cdata, scalars_in_return)),
    );
    context.insert(
        "NATIVE_LIBRARY_NAME".to_owned(),
        Value::String(options.native_library_name.to_owned()),
    );
    context.insert(
        "NATIVE_LIBRARY_VERSION".to_owned(),
        Value::String(options.native_library_version.to_owned()),
    );
    // Build-time metadata baked into the entity as constants. The compiled
    // bridge's PATH/HASH are unknown until it is built, so they are left empty
    // here and stamped in afterwards (see `stamp_entity_bridge`).
    context.insert(
        "DESCRIPTION".to_owned(),
        Value::String(php_single_quoted(options.description)),
    );
    context.insert("BRIDGE_PATH".to_owned(), Value::String(String::new()));
    context.insert("BRIDGE_HASH".to_owned(), Value::String(String::new()));
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

/// Escape a value for a PHP single-quoted string literal.
fn php_single_quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Fill the entity variants' `PATH`/`HASH` constants (left empty at generation
/// time) with the compiled bridge's path and content hash. Called after the
/// bridge is built (by `pnl install` and `pnlx build`).
pub fn stamp_entity_bridge(
    generated_dir: &Path,
    class_name: &str,
    bridge_path: &str,
    bridge_hash: &str,
) -> Result<()> {
    // The base entity and its three feature variants (cdata/, scalar/, cdata/scalar/).
    for variant in ["", "cdata", "scalar", "cdata/scalar"] {
        let file = generated_dir
            .join(variant)
            .join(format!("{class_name}.php"));
        if !file.is_file() {
            continue;
        }
        let content = fs::read_to_string(&file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let content = set_string_const(&content, "PATH", &php_single_quoted(bridge_path));
        let content = set_string_const(&content, "HASH", &php_single_quoted(bridge_hash));
        fs::write(&file, content).with_context(|| format!("failed to write {}", file.display()))?;
    }
    Ok(())
}

/// Replace the value of `public const string <name> = '<old>';` with `<value>`
/// (already escaped). No-op if the constant is not found.
fn set_string_const(content: &str, name: &str, value: &str) -> String {
    let prefix = format!("public const string {name} = '");
    let Some(start) = content.find(&prefix) else {
        return content.to_owned();
    };
    let value_start = start + prefix.len();
    let Some(rel_end) = content[value_start..].find('\'') else {
        return content.to_owned();
    };
    let end = value_start + rel_end;
    format!("{}{value}{}", &content[..value_start], &content[end..])
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

/// Render one of the static inner templates (methods, functions, aliases,
/// bridge) from a serializable context. These templates ship with the binary
/// and have fixed shapes, so a render failure is a build-time bug, not a runtime
/// condition — hence the panic rather than a propagated error.
fn render_inner_template(template: &str, context: Value) -> String {
    let mut handlebars = Handlebars::new();
    handlebars.register_escape_fn(handlebars::no_escape);
    handlebars
        .render_template(template, &context)
        .expect("generated inner template must render")
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
            native_library_name: "demo/demo",
            native_library_version: "1.0.0",
            description: "Demo native library.",
        }
    }

    #[test]
    fn renders_php_methods() {
        let signatures = sample_signatures();
        // Default variant: wrapper returns (scalars_in_return = false).
        insta::assert_snapshot!(render_methods(&sample_options(&signatures), false, false));
    }

    #[test]
    fn renders_php_methods_using_php_scalars() {
        let signatures = sample_signatures();
        // The `scalar/` variant: methods return PHP-native scalars.
        insta::assert_snapshot!(render_methods(&sample_options(&signatures), false, true));
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
