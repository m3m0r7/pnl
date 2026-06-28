use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{Map, Value};

use crate::model::platform::GeneratedMetadata;

mod aliases;
mod names;
mod parse;
mod php;
mod types;

use aliases::render_aliases;
use php::{render_global_functions, render_methods};

pub use parse::{
    FunctionSignature, StructField, apply_symbol_aliases, parse_function_signatures,
    parse_function_signatures_with_enums, parse_signatures_with_unsupported, parse_struct_fields,
};

const FFI_TEMPLATE: &str = include_str!("templates/package/src/generated/ffi.php.tpl");
const ENTITY_TEMPLATE: &str = include_str!("templates/package/src/generated/entity.php.tpl");
const COMPONENT_TEMPLATE: &str = include_str!("templates/package/src/generated/component.php.tpl");
const MANIFEST_TEMPLATE: &str = include_str!("templates/package/src/generated/manifest.php.tpl");
const CONTEXT_TEMPLATE: &str = include_str!("templates/package/src/generated/context.php.tpl");
const EXCEPTION_TEMPLATE: &str = include_str!("templates/package/src/generated/exception.php.tpl");
const TYPE_FILE_TEMPLATE: &str = include_str!("templates/package/src/generated/types.php.tpl");
const ENUM_FILE_TEMPLATE: &str = include_str!("templates/package/src/generated/enums.php.tpl");
const SYMBOL_TEMPLATE: &str = include_str!("templates/package/src/generated/symbol.php.tpl");
const CONST_TEMPLATE: &str = include_str!("templates/package/src/generated/const.php.tpl");
const MACRO_FUNCTIONS_TEMPLATE: &str =
    include_str!("templates/package/src/generated/macro.functions.php.tpl");
const AUTOLOAD_TEMPLATE: &str = include_str!("templates/workspace/autoload.php.tpl");
const IDE_HELPER_TEMPLATE: &str = include_str!("templates/workspace/ide-helper.php.tpl");
const INDEX_TEMPLATE: &str = include_str!("templates/package/src/generated/index.php.tpl");
const ALIASES_TEMPLATE: &str =
    include_str!("templates/package/src/generated/function.aliases.php.tpl");
const FUNCTIONS_TEMPLATE: &str = include_str!("templates/package/src/generated/functions.php.tpl");

// Inner templates: the repeated, per-symbol bodies that used to be assembled
// with `format!`/`push_str` in Rust now live here as Handlebars `{{#each}}`
// loops, so the generated layout is editable without touching code.
const METHODS_TEMPLATE: &str = include_str!("templates/partials/methods.php.tpl");
const GLOBAL_FUNCTIONS_TEMPLATE: &str = include_str!("templates/partials/global_functions.php.tpl");
const ALIASES_ENTRIES_TEMPLATE: &str = include_str!("templates/partials/aliases_entries.php.tpl");

pub fn generate_ffi_php_from_cdef(cdef: &str, out: &Path) -> Result<()> {
    let mut context = generated_template_context();
    context.insert("CDEF".to_owned(), Value::String(cdef.to_owned()));
    let generated = render_handlebars(FFI_TEMPLATE, context)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, generated).with_context(|| format!("failed to write {}", out.display()))?;
    crate::app::ui::created("generated", out);
    Ok(())
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
    /// Exported data symbols (C globals), surfaced as entity const markers plus
    /// per-symbol marker classes under `symbol/`.
    pub symbols: &'a [crate::native::header_adapter::DataSymbol],
    /// Named C enums surfaced as PHP `enum`s under `<namespace>\Enums`.
    pub enums: &'a [crate::native::header_adapter::EnumDef],
    /// Struct tag -> its fields, so a `Types\<tag>` wrapper gets typed accessors.
    pub struct_fields: &'a BTreeMap<String, Vec<StructField>>,
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

/// Generate one `<Class>LibraryComponent` trait variant (the method group the
/// entity mixes in). `allow_cdata`/`scalars_in_return` select the same per-variant
/// method shapes as {@see generate_entity_php}.
pub fn generate_component_php(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
    scalars_in_return: bool,
) -> Result<()> {
    write_template(
        out,
        COMPONENT_TEMPLATE,
        options,
        allow_cdata,
        scalars_in_return,
    )
}

/// Generate the `<Class>Manifest` metadata class.
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
    generated_dir: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    constants: &[crate::native::header_adapter::Constant],
) -> Result<()> {
    // Two variants, picked at runtime by `index.php` from `use_php_scalars_in_const`
    // (the same always-generate-both approach as the entity's `scalar/` variant):
    // the default wraps every value in a `\Pnlx\Types\*`; `scalar/const.php` uses a
    // bare PHP scalar where one represents the value losslessly.
    write_const_variant(&generated_dir.join("const.php"), options, constants, false)?;
    write_const_variant(
        &generated_dir.join("scalar").join("const.php"),
        options,
        constants,
        true,
    )
}

fn write_const_variant(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    constants: &[crate::native::header_adapter::Constant],
    scalars: bool,
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
                .map(|constant| {
                    let value = if scalars {
                        &constant.scalar
                    } else {
                        &constant.wrapped
                    };
                    serde_json::json!({ "name": constant.name, "value": value })
                })
                .collect(),
        ),
    );
    write_generated(out, render_handlebars(CONST_TEMPLATE, context)?)
}

/// Generate the per-symbol marker classes into `dir` (`src/generated/symbol/`), one
/// per exported data symbol. Each is a flat `\<Ns>\<name>` class implementing
/// `\Pnlx\FFI\SymbolInterface`, passed straight to a function and resolved by the
/// argument marshaller. Returns the written class names.
pub fn generate_symbols_php(
    dir: &Path,
    options: &PhpPackageTemplateOptions<'_>,
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for symbol in options.symbols {
        let mut context = generated_template_context();
        context.insert(
            "NAMESPACE".to_owned(),
            Value::String(options.namespace.to_owned()),
        );
        context.insert("SYMBOL".to_owned(), Value::String(symbol.name.clone()));
        context.insert(
            "ENTITY".to_owned(),
            Value::String(format!("\\{}\\{}", options.namespace, options.class_name)),
        );
        context.insert(
            "MODE".to_owned(),
            Value::String(if symbol.pointer { "Value" } else { "Address" }.to_owned()),
        );
        write_generated(
            &dir.join(format!("{}.php", symbol.name)),
            render_handlebars(SYMBOL_TEMPLATE, context)?,
        )?;
        names.push(symbol.name.clone());
    }
    Ok(names)
}

/// Generate the per-package PHP enums into `dir` (`src/generated/enums/`), one
/// int-backed `enum` per file. Returns the enum class names written.
pub fn generate_enums_php(
    dir: &Path,
    options: &PhpPackageTemplateOptions<'_>,
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for def in options.enums {
        let class = php::enum_class_name(&def.name);
        let mut context = generated_template_context();
        context.insert(
            "NAMESPACE".to_owned(),
            Value::String(options.namespace.to_owned()),
        );
        context.insert("NAME".to_owned(), Value::String(def.name.clone()));
        context.insert("ENUM_CLASS".to_owned(), Value::String(class.clone()));
        context.insert(
            "cases".to_owned(),
            Value::Array(
                def.cases
                    .iter()
                    .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
                    .collect(),
            ),
        );
        write_generated(
            &dir.join(format!("{class}.php")),
            render_handlebars(ENUM_FILE_TEMPLATE, context)?,
        )?;
        names.push(class);
    }
    Ok(names)
}

/// Generate the per-package pointer wrappers into `dir` (`src/generated/types/`),
/// one class per file. Returns the type class names that were written (so the
/// package's `index.php` can require each one).
pub fn generate_types_php(
    dir: &Path,
    options: &PhpPackageTemplateOptions<'_>,
) -> Result<Vec<String>> {
    let base = format!("\\{}\\{}Context", options.namespace, options.class_name);
    // PHP class names are case-insensitive, so two collected type names differing
    // only in case — a C struct tag like `Argon2_Context` and its typedef
    // `argon2_context` — declare the same class. They collapse to one file on a
    // case-insensitive filesystem (macOS) but become two colliding files on a
    // case-sensitive one (Linux), where the index glob requires both and PHP fatals
    // with "cannot declare class ... already in use". Emit one wrapper per
    // case-insensitive name, preferring the spelling that carries struct fields so
    // the typed accessors survive (the fieldless typedef otherwise wins by file
    // order). References to either spelling resolve to it, since PHP is case-blind.
    let mut by_ci: BTreeMap<String, String> = BTreeMap::new();
    for name in php::collect_pointer_types(options) {
        let key = name.to_ascii_lowercase();
        let prefer = match by_ci.get(&key) {
            None => true,
            Some(existing) => {
                options.struct_fields.contains_key(&name)
                    && !options.struct_fields.contains_key(existing)
            }
        };
        if prefer {
            by_ci.insert(key, name);
        }
    }
    let types: Vec<String> = by_ci.into_values().collect();
    for type_name in &types {
        let mut context = generated_template_context();
        context.insert(
            "NAMESPACE".to_owned(),
            Value::String(options.namespace.to_owned()),
        );
        context.insert("BASE".to_owned(), Value::String(base.clone()));
        context.insert(
            "ENTITY".to_owned(),
            Value::String(format!("\\{}\\{}", options.namespace, options.class_name)),
        );
        context.insert("TYPE".to_owned(), Value::String(type_name.clone()));
        // Typed field accessors when this pointer type names a struct the cdef
        // defines with a body; an opaque pointer type just gets the bare wrapper.
        let fields = options
            .struct_fields
            .get(type_name)
            .map(|fields| php::struct_field_views(fields, options))
            .unwrap_or_default();
        context.insert("fields".to_owned(), Value::Array(fields));
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
    composites: &[String],
    manifest_path: &str,
) -> Result<()> {
    let mut context = generated_template_context();
    context.insert("VERSION".to_owned(), Value::String(version.to_owned()));
    context.insert(
        "PACKAGES".to_owned(),
        serde_json::to_value(packages).expect("package paths serialize to JSON"),
    );
    context.insert(
        "COMPOSITES".to_owned(),
        serde_json::to_value(composites).expect("composite paths serialize to JSON"),
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
        Value::String(crate::model::config::SCHEMA_VERSION.to_owned()),
    );
    context.insert(
        "CONFIG_SELF_REPOSITORY".to_owned(),
        Value::String(crate::model::config::SELF_REPOSITORY.to_owned()),
    );
    context.insert(
        "CONFIG_PACKAGES_REPOSITORY".to_owned(),
        Value::String(crate::model::config::PACKAGES_REPOSITORY.to_owned()),
    );
    context.insert(
        "CONFIG_OUTPUT_DIR".to_owned(),
        Value::String(crate::model::config::DEFAULT_OUTPUT_DIR.to_owned()),
    );
    context.insert(
        "CONFIG_TTL_SECONDS".to_owned(),
        Value::Number(crate::model::config::UPDATE_CHECK_TTL_SECONDS.into()),
    );
    context.insert(
        "CONFIG_OPT_OUT_ENV".to_owned(),
        Value::String(crate::model::config::UPDATE_CHECK_OPT_OUT_ENV.to_owned()),
    );
    context.insert(
        "CONFIG_CACHE_KEY".to_owned(),
        Value::String(crate::model::config::UPDATE_CHECK_CACHE_KEY.to_owned()),
    );
    context.insert(
        "CONFIG_BINARIES".to_owned(),
        serde_json::to_value(crate::model::config::BINARIES).expect("binaries serialize to JSON"),
    );
    context.insert(
        "BUILD_OS".to_owned(),
        Value::String(crate::model::config::BUILD_OS.to_owned()),
    );
    context.insert(
        "BUILD_ARCH".to_owned(),
        Value::String(crate::model::config::BUILD_ARCH.to_owned()),
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
    crate::app::ui::created("generated", out);
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
    crate::app::ui::created("generated", out);
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
    crate::app::ui::created("generated", out);
    Ok(())
}

/// Generate `macro.functions.php`: function-like C macros surfaced as PHP
/// functions under `\Pnlx\Func\<Class>`. Always loaded.
pub fn generate_macro_functions_php(
    out: &Path,
    options: &PhpPackageTemplateOptions<'_>,
    macro_functions: &[crate::native::header_adapter::MacroFunction],
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
    macro_functions: &[crate::native::header_adapter::MacroFunction],
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
            "if (!function_exists('{fqn}')) {{\n    #[\\Pnlx\\Attribute\\AutoGeneratedByPnlx]\n    #[\\Pnlx\\Attribute\\RawNativeName('{}')]\n    function {}({params})\n    {{\n{body}\n    }}\n}}\n\n",
            function.name, function.name
        ));
    }
    out
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
    crate::app::ui::created("generated", out);
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
    // The boot token is derived from the same native-library hash as the HASH
    // constant and is filled in beside PATH/HASH once the library is resolved.
    context.insert("BOOT_TOKEN".to_owned(), Value::String(String::new()));
    context.insert(
        "NATIVE_LIBRARY_NAME".to_owned(),
        Value::String(options.native_library_name.to_owned()),
    );
    context.insert(
        "NATIVE_LIBRARY_VERSION".to_owned(),
        Value::String(options.native_library_version.to_owned()),
    );
    // Build-time metadata baked into the entity as constants. The resolved
    // native library PATH/HASH are stamped in after generation.
    context.insert(
        "DESCRIPTION".to_owned(),
        Value::String(php_single_quoted(options.description)),
    );
    context.insert("NATIVE_PATH".to_owned(), Value::String(String::new()));
    context.insert("NATIVE_HASH".to_owned(), Value::String(String::new()));
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

/// Stamp the resolved native library `path`/`hash`/`libraries` into the generated
/// entity's `#[\Pnlx\Attribute\NativeLibrary(...)]` attribute. The entity is a
/// single variant-independent file (the per-variant method shapes live in the
/// `<Class>LibraryComponent` trait), so there is one file to update.
pub fn stamp_entity_native_library(
    generated_dir: &Path,
    class_name: &str,
    native_path: &str,
    native_hash: &str,
    dependency_libraries: &[String],
) -> Result<()> {
    let file = generated_dir.join(format!("{class_name}.php"));
    if !file.is_file() {
        return Ok(());
    }
    let content =
        fs::read_to_string(&file).with_context(|| format!("failed to read {}", file.display()))?;
    let content = set_attribute_string_arg(&content, "path", &php_single_quoted(native_path));
    let content = set_attribute_string_arg(&content, "hash", &php_single_quoted(native_hash));
    let content = set_attribute_array_arg(&content, "libraries", dependency_libraries);
    fs::write(&file, content).with_context(|| format!("failed to write {}", file.display()))?;
    Ok(())
}

/// Replace the value of a single-quoted named attribute argument `<name>: '<old>'`
/// with `<value>` (already escaped). No-op if the argument is not found.
fn set_attribute_string_arg(content: &str, name: &str, value: &str) -> String {
    let prefix = format!("{name}: '");
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

/// Replace the `[...]` value of a named attribute argument `<name>: [...]` with a
/// PHP array literal of the given single-quoted strings (co-load library paths).
fn set_attribute_array_arg(content: &str, name: &str, items: &[String]) -> String {
    let prefix = format!("{name}: ");
    let Some(start) = content.find(&prefix) else {
        return content.to_owned();
    };
    let value_start = start + prefix.len();
    let Some(rel_end) = content[value_start..].find(']') else {
        return content.to_owned();
    };
    let end = value_start + rel_end + 1;
    let literal = if items.is_empty() {
        "[]".to_owned()
    } else {
        let joined = items
            .iter()
            .map(|item| format!("'{}'", php_single_quoted(item)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{joined}]")
    };
    format!("{}{literal}{}", &content[..value_start], &content[end..])
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
/// snippets) from a serializable context. These templates ship with the binary
/// and have fixed shapes, so a render failure is a build-time bug, not a runtime
/// condition — hence the panic rather than a propagated error.
fn render_inner_template(template: &str, context: Value) -> String {
    let mut handlebars = Handlebars::new();
    handlebars.register_escape_fn(handlebars::no_escape);
    handlebars
        .render_template(template, &context)
        .expect("generated inner template must render")
}

#[cfg(test)]
mod tests {
    use super::parse::FunctionParam;
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
            symbols: &[],
            enums: &[],
            struct_fields: {
                // An empty static keeps the borrow `'static` (a `&BTreeMap::new()`
                // temporary would not outlive the function).
                static EMPTY_FIELDS: BTreeMap<String, Vec<StructField>> = BTreeMap::new();
                &EMPTY_FIELDS
            },
        }
    }

    #[test]
    fn renders_php_methods() {
        let signatures = sample_signatures();
        // Default variant: wrapper returns (scalars_in_return = false).
        insta::assert_snapshot!(render_methods(&sample_options(&signatures), false, false,));
    }

    #[test]
    fn out_parameter_param_accepts_wrappers_and_gates_cdata() {
        let signatures = parse_function_signatures("void demo_version(int *major, int *minor);\n");
        let options = sample_options(&signatures);
        let without_cdata = render_methods(&options, false, false);
        let with_cdata = render_methods(&options, true, false);

        // The scalar-pointer out-param is a by-reference NativePointer that also
        // accepts the integer helper wrapper, in both variants.
        assert!(
            without_cdata.contains("#[\\Pnlx\\Attribute\\NativePointer('int')]"),
            "{without_cdata}"
        );
        assert!(
            without_cdata.contains("int|\\Pnlx\\Types\\AnySizeInteger|array|null &$major"),
            "{without_cdata}"
        );
        // `\FFI\CData` is only offered when allow_cdata is set — never leaked into
        // the scalar (non-cdata) variant.
        assert!(
            !without_cdata.contains("\\FFI\\CData"),
            "CData leaked into the non-cdata variant: {without_cdata}"
        );
        assert!(
            with_cdata
                .contains("int|\\Pnlx\\Types\\AnySizeInteger|array|\\FFI\\CData|null &$major"),
            "{with_cdata}"
        );
    }

    #[test]
    fn renders_php_methods_using_php_scalars() {
        let signatures = sample_signatures();
        // The `scalar/` variant: methods return PHP-native scalars.
        insta::assert_snapshot!(render_methods(&sample_options(&signatures), false, true,));
    }

    #[test]
    fn renders_php_global_functions() {
        let signatures = sample_signatures();
        insta::assert_snapshot!(render_global_functions(&sample_options(&signatures)));
    }

    #[test]
    fn stamps_native_library_attribute_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path();
        let entity = generated.join("Demo.php");
        fs::write(
            &entity,
            "<?php\n#[\\Pnlx\\Attribute\\NativeLibrary(\n    name: 'demo/demo',\n    path: '',\n    hash: '',\n    libraries: [],\n)]\nclass Demo extends \\Pnlx\\Extension\\AbstractExtension {}\n",
        )
        .unwrap();

        stamp_entity_native_library(
            generated,
            "Demo",
            "/usr/lib/libdemo.dylib",
            "abc123",
            &["/usr/lib/libcblas.so".to_owned()],
        )
        .unwrap();

        let stamped = fs::read_to_string(entity).unwrap();
        assert!(
            stamped.contains("path: '/usr/lib/libdemo.dylib'"),
            "{stamped}"
        );
        assert!(stamped.contains("hash: 'abc123'"), "{stamped}");
        assert!(
            stamped.contains("libraries: ['/usr/lib/libcblas.so']"),
            "{stamped}"
        );
    }

    #[test]
    fn skips_reserved_php_global_function_names() {
        let signatures = parse_function_signatures("void exit(int status);\nint demo_ok(void);\n");
        let rendered = render_global_functions(&sample_options(&signatures));
        assert!(!rendered.contains("function exit("), "{rendered}");
        assert!(rendered.contains("function demo_ok("), "{rendered}");
    }

    #[test]
    fn renders_php_aliases() {
        insta::assert_snapshot!(render_aliases(&sample_signatures()));
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

    #[test]
    fn parses_variadic_and_function_pointer_parameters() {
        let cdef = "int printf(const char *format, ...);\n\
void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));\n";
        let signatures = parse_function_signatures(cdef);

        assert_eq!(signatures[0].name, "printf");
        assert!(signatures[0].variadic);
        assert_eq!(
            signatures[0].params,
            vec![FunctionParam {
                name: "format".to_owned(),
                type_name: "const char *".to_owned(),
                callback: false,
                enum_type: None,
            }]
        );
        assert_eq!(signatures[1].name, "qsort");
        assert!(!signatures[1].variadic);
        assert_eq!(
            signatures[1].params[3],
            FunctionParam {
                name: "compar".to_owned(),
                type_name: "void *".to_owned(),
                callback: true,
                enum_type: None,
            }
        );
    }

    #[test]
    fn parses_struct_fields_with_bodies_for_accessors() {
        // A struct with a body yields its fields (types resolved through typedefs);
        // an opaque (body-less) struct yields nothing; a nested aggregate/function
        // field is skipped so the accessor degrades rather than mis-parsing.
        let cdef = "typedef unsigned long size_t;\n\
struct ex_point { int x; int y; };\n\
struct ex_box { struct ex_point *origin; char *label; size_t len; };\n\
typedef struct ex_opaque ex_opaque;\n";
        let fields = parse_struct_fields(cdef);

        assert_eq!(
            fields.get("ex_point"),
            Some(&vec![
                StructField {
                    name: "x".to_owned(),
                    type_name: "int".to_owned(),
                    wide_string: false,
                },
                StructField {
                    name: "y".to_owned(),
                    type_name: "int".to_owned(),
                    wide_string: false,
                },
            ])
        );
        // `size_t` resolves through the typedef to `unsigned long`; the struct
        // pointer and char pointer keep their pointer spelling.
        let box_fields = fields.get("ex_box").expect("ex_box parsed");
        assert_eq!(box_fields[0].type_name, "struct ex_point *");
        assert_eq!(box_fields[1].type_name, "char *");
        assert_eq!(box_fields[2].type_name, "unsigned long");
        // An opaque struct (no body) is not in the map.
        assert!(!fields.contains_key("ex_opaque"));
    }

    #[test]
    fn struct_fields_skip_inline_union_members() {
        // luaL_Buffer's shape: an inline anonymous union. Splitting on top-level `;`
        // only must keep the union as ONE (skipped) member, NOT harvest its inner
        // members as fields — otherwise the union's `long l` would collide with the
        // real `lua_State *L` (both → `getL`).
        let cdef = "struct ex_buf { char *b; long n; void *L; \
union { long l; double u; char b[8]; } init; };\n";
        let fields = parse_struct_fields(cdef).remove("ex_buf").expect("parsed");
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        // Only the top-level members; the union's inner `l`/`u`/`b`/`init` are gone.
        assert_eq!(names, vec!["b", "n", "L"], "fields: {names:?}");
    }

    #[test]
    fn resolves_char_pointer_typedefs_to_const_char_pointer() {
        // libtidy's `ctmbstr` -> `const tmbchar *` -> `char *`, and libpng's
        // `png_const_charp` -> `const char *`. Both should be rewritten to
        // `const char *` so the PHP layer accepts a string; a pointer-to-pointer
        // typedef must NOT be rewritten.
        let cdef = "typedef char tmbchar;\n\
typedef const tmbchar *ctmbstr;\n\
typedef const char *png_const_charp;\n\
typedef char **png_charpp;\n\
int tidyParseString(int doc, ctmbstr content);\n\
int png_write(png_const_charp name, png_charpp rows);\n";
        let signatures = parse_function_signatures(cdef);

        let parse = &signatures[0];
        assert_eq!(parse.params[1].type_name, "const char *");

        let write = &signatures[1];
        assert_eq!(write.params[0].type_name, "const char *");
        // `png_charpp` is `char **`; it stays a real pointer-to-pointer.
        assert_eq!(write.params[1].type_name, "png_charpp");
    }

    #[test]
    fn resolves_pointer_typedef_depth_for_handles_and_struct_pointers() {
        // A typedef to a struct pointer hides one `*`: revealing it makes
        // `FT_Library *` a handle out-param (`**`) and gives `mpz_ptr`'s pointee a
        // wrapper. A pointer-to-byte typedef (`char **`) must stay opaque.
        let cdef = "typedef struct FT_LibraryRec_ *FT_Library;\n\
typedef struct __mpz_struct __mpz_struct;\n\
typedef __mpz_struct *mpz_ptr;\n\
typedef char **png_charpp;\n\
int FT_Init_FreeType(FT_Library *alibrary);\n\
void FT_Done_FreeType(FT_Library library);\n\
void mpz_init(mpz_ptr x);\n\
void png_rows(png_charpp rows);\n";
        let sigs = parse_function_signatures(cdef);
        assert_eq!(sigs[0].params[0].type_name, "struct FT_LibraryRec_ **");
        assert_eq!(sigs[1].params[0].type_name, "struct FT_LibraryRec_ *");
        assert_eq!(sigs[2].params[0].type_name, "__mpz_struct *");
        // `png_charpp` (= `char **`) is a byte pointer-to-pointer: left untouched.
        assert_eq!(sigs[3].params[0].type_name, "png_charpp");
    }

    #[test]
    fn resolves_byte_typedef_inside_a_pointer_to_a_string() {
        // A pointer to a scalar byte typedef (oniguruma `const OnigUChar *`,
        // pcre2 `PCRE2_SPTR8`) resolves its element so is_char_pointer sees it.
        let cdef = "typedef unsigned char OnigUChar;\n\
typedef const unsigned char *PCRE2_SPTR8;\n\
int onig_new(const OnigUChar *pattern);\n\
int pcre2_compile(PCRE2_SPTR8 pattern);\n";
        let signatures = parse_function_signatures(cdef);
        // `const OnigUChar *` → element resolved to `unsigned char`.
        assert_eq!(signatures[0].params[0].type_name, "const unsigned char *");
        // `PCRE2_SPTR8` (a pointer typedef) → rewritten to `const char *`.
        assert_eq!(signatures[1].params[0].type_name, "const char *");
    }

    #[test]
    fn symbol_alias_adds_public_name_targeting_versioned_symbol() {
        // The cdef declares the versioned export; the alias adds a public-name method
        // (dispatch + alias map target the export) while keeping the raw-name method.
        let mut signatures = parse_function_signatures("const char *u_errorName_74(int code);\n");
        apply_symbol_aliases(
            &mut signatures,
            &[("u_errorName".to_owned(), "u_errorName_74".to_owned())],
        );
        // Both the raw export and the public alias are present.
        assert_eq!(signatures.len(), 2);
        assert_eq!(signatures[0].name, "u_errorName_74");
        assert_eq!(signatures[0].native_symbol(), "u_errorName_74");
        assert_eq!(signatures[1].name, "u_errorName");
        assert_eq!(signatures[1].native_symbol(), "u_errorName_74");

        // The alias map keys both names to the exported symbol.
        let aliases = super::aliases::render_aliases(&signatures);
        assert!(
            aliases.contains("'u_errorName' => 'u_errorName_74'"),
            "{aliases}"
        );
    }

    #[test]
    fn symbol_alias_skips_when_public_name_already_exists() {
        // An alias must never shadow a real export of the public name.
        let mut signatures =
            parse_function_signatures("void u_foo(int code);\nvoid u_foo_74(int code);\n");
        apply_symbol_aliases(
            &mut signatures,
            &[("u_foo".to_owned(), "u_foo_74".to_owned())],
        );
        assert_eq!(signatures.len(), 2);
        assert!(signatures.iter().all(|sig| sig.native_symbol.is_none()));
    }
}
