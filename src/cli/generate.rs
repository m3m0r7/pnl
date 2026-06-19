use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{Map, Value};

use crate::platform::GeneratedMetadata;

mod aliases;
mod names;
mod php;
mod types;

use aliases::render_aliases;
use php::{render_global_functions, render_methods};
use types::sanitize_php_param_name;

const FFI_TEMPLATE: &str = include_str!("templates/package/src/generated/ffi.php.tpl");
const ENTITY_TEMPLATE: &str = include_str!("templates/package/src/generated/entity.php.tpl");
const MANIFEST_TEMPLATE: &str = include_str!("templates/package/src/generated/manifest.php.tpl");
const CONTEXT_TEMPLATE: &str = include_str!("templates/package/src/generated/context.php.tpl");
const EXCEPTION_TEMPLATE: &str = include_str!("templates/package/src/generated/exception.php.tpl");
const TYPE_FILE_TEMPLATE: &str = include_str!("templates/package/src/generated/types.php.tpl");
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
    crate::ui::created("generated", out);
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
    pub symbols: &'a [crate::header_adapter::DataSymbol],
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
    constants: &[crate::header_adapter::Constant],
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
    constants: &[crate::header_adapter::Constant],
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
        context.insert(
            "ENTITY".to_owned(),
            Value::String(format!("\\{}\\{}", options.namespace, options.class_name)),
        );
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
        Value::String(render_methods(options, allow_cdata, scalars_in_return, "")),
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

/// Fill the entity variants' `PATH`/`HASH` constants with the resolved native
/// library path and content hash.
pub fn stamp_entity_native_library(
    generated_dir: &Path,
    class_name: &str,
    native_path: &str,
    native_hash: &str,
    dependency_libraries: &[String],
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
        let content = set_string_const(&content, "PATH", &php_single_quoted(native_path));
        let content = set_string_const(&content, "HASH", &php_single_quoted(native_hash));
        let content = set_array_const(&content, "LIBRARIES", dependency_libraries);
        let content = set_string_const(
            &content,
            "PNLX_BOOT_TOKEN",
            &php_single_quoted(&format!("__pnlx_boot_{native_hash}")),
        );
        fs::write(&file, content).with_context(|| format!("failed to write {}", file.display()))?;
    }
    Ok(())
}

/// Replace the value of `<visibility> const string <name> = '<old>';` with
/// `<value>` (already escaped). No-op if the constant is not found.
/// Replace an `public const array <name> = [...];` value with a PHP array literal
/// of the given strings (each single-quoted). Used to stamp the resolved co-load
/// library paths into the generated entity.
fn set_array_const(content: &str, name: &str, items: &[String]) -> String {
    let prefix = format!("public const array {name} = ");
    let Some(start) = content.find(&prefix) else {
        return content.to_owned();
    };
    let value_start = start + prefix.len();
    let Some(rel_end) = content[value_start..].find(';') else {
        return content.to_owned();
    };
    let end = value_start + rel_end;
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

fn set_string_const(content: &str, name: &str, value: &str) -> String {
    for visibility in ["public", "protected"] {
        let prefix = format!("{visibility} const string {name} = '");
        let Some(start) = content.find(&prefix) else {
            continue;
        };
        let value_start = start + prefix.len();
        let Some(rel_end) = content[value_start..].find('\'') else {
            return content.to_owned();
        };
        let end = value_start + rel_end;
        return format!("{}{value}{}", &content[..value_start], &content[end..]);
    }
    content.to_owned()
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    pub name: String,
    pub(super) return_type: String,
    pub(super) params: Vec<FunctionParam>,
    pub variadic: bool,
    /// The C symbol FFI dispatches to, when it differs from the public `name`.
    /// Set for symbol-version renames (ICU's `u_errorName` method dispatches to the
    /// versioned export `u_errorName_74`); `None` when the name *is* the symbol.
    pub(super) native_symbol: Option<String>,
    /// When set, this function has no FFI binding and its generated method throws
    /// with this reason (e.g. `static inline`) instead of dispatching. It is not in
    /// the cdef and is excluded from the alias map and the global-functions API.
    pub(super) unsupported: Option<String>,
}

impl FunctionSignature {
    /// The C symbol the library exports — the rename target when one is set,
    /// otherwise the public name itself.
    pub(super) fn native_symbol(&self) -> &str {
        self.native_symbol.as_deref().unwrap_or(&self.name)
    }
}

/// Expose a public method name for an export reached through a symbol-version
/// alias. For each `(public_name, native_symbol)` pair, a fully-marshalled clone of
/// the export's signature is added under the public name (`mpz_init` alongside
/// `__gmpz_init`, `u_errorName` alongside `u_errorName_74`), keeping the original
/// raw-name method so existing callers of either name keep working. The clone's
/// `native_symbol` makes dispatch and the alias map target the real export. A pair
/// is skipped when the public name already names a real export or another alias.
pub fn apply_symbol_aliases(signatures: &mut Vec<FunctionSignature>, aliases: &[(String, String)]) {
    if aliases.is_empty() {
        return;
    }
    let mut taken: BTreeSet<String> = signatures.iter().map(|sig| sig.name.clone()).collect();
    let mut additions = Vec::new();
    for (public, native) in aliases {
        if taken.contains(public) {
            continue;
        }
        if let Some(base) = signatures.iter().find(|sig| &sig.name == native) {
            let mut clone = base.clone();
            clone.name = public.clone();
            clone.native_symbol = Some(native.clone());
            additions.push(clone);
            taken.insert(public.clone());
        }
    }
    signatures.extend(additions);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FunctionParam {
    pub(super) name: String,
    pub(super) type_name: String,
}

pub fn parse_function_signatures(cdef: &str) -> Vec<FunctionSignature> {
    let raw_typedefs = raw_typedef_map(cdef);
    let scalar_typedefs = scalar_typedef_map(&raw_typedefs);
    let char_pointer_typedefs = char_pointer_typedef_set(&raw_typedefs);
    let mut seen = BTreeSet::new();
    cdef.lines()
        .filter_map(parse_function_signature)
        .map(|mut signature| {
            signature.return_type =
                resolve_scalar_typedef(&signature.return_type, &scalar_typedefs);
            signature.return_type =
                resolve_char_pointer_typedef(&signature.return_type, &char_pointer_typedefs);
            signature.return_type = resolve_pointer_typedef(&signature.return_type, &raw_typedefs);
            for param in &mut signature.params {
                param.type_name = resolve_scalar_typedef(&param.type_name, &scalar_typedefs);
                param.type_name =
                    resolve_char_pointer_typedef(&param.type_name, &char_pointer_typedefs);
                param.type_name = resolve_pointer_typedef(&param.type_name, &raw_typedefs);
            }
            signature
        })
        .filter(|signature| seen.insert(signature.name.clone()))
        .collect()
}

/// Parse the cdef's function signatures and append the `unsupported` functions
/// (which are NOT in the cdef, e.g. `static inline`), tagging each so its generated
/// method throws instead of dispatching. The unsupported declarations are parsed
/// alongside the cdef so their parameter/return types resolve against the same
/// typedefs and the stub gets a faithful signature.
pub fn parse_signatures_with_unsupported(
    cdef: &str,
    unsupported: &[crate::header_adapter::UnsupportedFunction],
) -> Vec<FunctionSignature> {
    if unsupported.is_empty() {
        return parse_function_signatures(cdef);
    }
    let reason_by_name: BTreeMap<String, String> = unsupported
        .iter()
        .filter_map(|function| {
            parse_function_signature(&function.declaration)
                .map(|signature| (signature.name, function.reason.clone()))
        })
        .collect();
    let declarations = unsupported
        .iter()
        .map(|function| function.declaration.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut signatures = parse_function_signatures(&format!("{cdef}\n{declarations}"));
    for signature in &mut signatures {
        if let Some(reason) = reason_by_name.get(&signature.name) {
            signature.unsupported = Some(reason.clone());
        }
    }
    signatures
}

/// All `typedef <underlying> <name>;` pairs in the cdef (excluding
/// function-pointer typedefs), as raw `name -> underlying` strings.
fn raw_typedef_map(cdef: &str) -> BTreeMap<String, String> {
    let mut raw = BTreeMap::new();
    for line in cdef.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("typedef ") else {
            continue;
        };
        let Some(rest) = rest.strip_suffix(';') else {
            continue;
        };
        // Skip function-pointer typedefs (`typedef ret (*name)(...)`).
        if rest.contains('(') {
            continue;
        }
        if let Some((underlying, name)) = split_c_declaration_name(rest) {
            raw.insert(name, underlying);
        }
    }
    raw
}

/// Map of typedef name -> underlying builtin scalar, for the cdef's simple
/// scalar typedefs (e.g. zlib's `uLong` -> `unsigned long`). Used so a parameter
/// typed as an integer/float typedef is recognised as a PHP scalar under
/// `use_php_scalars_in_params`, instead of demanding a wrapper object.
fn scalar_typedef_map(raw: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut resolved = BTreeMap::new();
    for name in raw.keys() {
        if let Some(scalar) = resolve_typedef_to_scalar(name, raw, 0) {
            resolved.insert(name.clone(), scalar);
        }
    }
    resolved
}

/// Byte-pointer base types whose single-level pointer is passed as a PHP string
/// (kept in sync with [`types::is_char_pointer`]).
const CHAR_POINTER_BASES: &[&str] = &["char", "unsigned char", "signed char", "uint8_t", "int8_t"];

/// Typedef names that resolve to a single-level pointer to a byte type
/// (e.g. libtidy's `ctmbstr` -> `const tmbchar *` -> `char *`, libpng's
/// `png_const_charp` -> `const char *`). A parameter/return typed as one of these
/// is rewritten to `const char *` so the PHP layer accepts a string, matching how
/// the C API and the examples treat them.
fn char_pointer_typedef_set(raw: &BTreeMap<String, String>) -> BTreeSet<String> {
    raw.keys()
        .filter(|name| resolves_to_char_pointer(name, raw, 0))
        .cloned()
        .collect()
}

fn resolves_to_char_pointer(name: &str, raw: &BTreeMap<String, String>, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    let Some(underlying) = raw.get(name) else {
        return false;
    };
    match underlying.matches('*').count() {
        // A single-level pointer: its element type must be a byte type, either a
        // builtin or another typedef that resolves to one (`tmbchar` -> `char`).
        1 => {
            let base = normalize_underlying(&underlying.replace('*', " "));
            if CHAR_POINTER_BASES.contains(&base.as_str()) {
                return true;
            }
            base.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                && resolve_typedef_to_scalar(&base, raw, depth + 1)
                    .is_some_and(|scalar| CHAR_POINTER_BASES.contains(&scalar.as_str()))
        }
        // A plain alias to another typedef (`ctmbstr2` -> `ctmbstr`): follow it.
        0 if underlying
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_') =>
        {
            resolves_to_char_pointer(underlying, raw, depth + 1)
        }
        _ => false,
    }
}

/// Strip `const`/`volatile`/`restrict` qualifiers and collapse whitespace.
fn normalize_underlying(underlying: &str) -> String {
    underlying
        .split_whitespace()
        .filter(|token| !matches!(*token, "const" | "volatile" | "restrict"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replace a char-pointer typedef used directly (not as a pointer-to-pointer)
/// with `const char *`, so the PHP type layer treats it as a string.
fn resolve_char_pointer_typedef(type_name: &str, set: &BTreeSet<String>) -> String {
    if set.contains(type_name.trim()) {
        "const char *".to_owned()
    } else {
        type_name.to_owned()
    }
}

fn resolve_typedef_to_scalar(
    name: &str,
    raw: &BTreeMap<String, String>,
    depth: usize,
) -> Option<String> {
    if depth > 16 {
        return None;
    }
    let underlying = raw.get(name)?;
    // A pointer/array typedef is not a value scalar.
    if underlying.contains('*') || underlying.contains('[') {
        return None;
    }
    if types::scalar_wrapper(underlying).is_some() {
        return Some(underlying.clone());
    }
    // The underlying may itself be another simple typedef (e.g. `Bytef` -> `Byte`
    // -> `unsigned char`); follow single-identifier chains.
    let core = underlying.trim();
    if core
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return resolve_typedef_to_scalar(core, raw, depth + 1);
    }
    None
}

/// Replace a scalar typedef name with its builtin underlying, so the PHP type
/// layer sees the real type. A by-value typedef becomes a scalar; a single-level
/// pointer's element is resolved too (`const OnigUChar *` → `const unsigned char *`,
/// `PCRE2_SPTR` → `const uint8_t *`) so a pointer to a byte typedef is recognised
/// as a string by [`types::is_char_pointer`]. Arrays and pointer-to-pointer are
/// left untouched (they stay real pointers).
fn resolve_scalar_typedef(type_name: &str, map: &BTreeMap<String, String>) -> String {
    if type_name.contains('[') || type_name.matches('*').count() > 1 {
        return type_name.to_owned();
    }
    type_name
        .split_whitespace()
        .map(|token| map.get(token).map(String::as_str).unwrap_or(token))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve a *pointer* typedef to its underlying base type plus the number of `*`
/// levels it hides — but only when the typedef actually denotes a pointer.
/// `FT_Library` (= `FT_LibraryRec_ *`) yields `("struct FT_LibraryRec_", 1)`;
/// `mpz_ptr` (= `__mpz_struct *`) yields `("__mpz_struct", 1)`. A struct/scalar
/// alias (`config_t` -> `struct config_t`) is not a pointer, so returns `None` and
/// is left untouched. Plain aliases to another pointer typedef are followed.
fn resolve_pointer_typedef_base(
    name: &str,
    raw: &BTreeMap<String, String>,
    depth: usize,
) -> Option<(String, usize)> {
    if depth > 16 {
        return None;
    }
    let underlying = raw.get(name)?;
    if underlying.contains('[') {
        return None;
    }
    let stars = underlying.matches('*').count();
    let base = normalize_underlying(&underlying.replace('*', " "))
        .trim()
        .to_owned();
    if stars == 0 {
        // A plain alias to another typedef — follow it only if THAT is a pointer.
        if base
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return resolve_pointer_typedef_base(&base, raw, depth + 1);
        }
        return None;
    }
    // The base may itself be a pointer typedef (`typedef Inner* Mid; typedef Mid* X;`).
    if let Some((inner, inner_stars)) = resolve_pointer_typedef_base(&base, raw, depth + 1) {
        return Some((inner, stars + inner_stars));
    }
    Some((base, stars))
}

/// Expand a pointer typedef in a parameter/return type so its real pointer depth
/// shows: `FT_Library *` becomes `struct FT_LibraryRec_ **` (a handle out-param),
/// `mpz_ptr` becomes `__mpz_struct *` (so the pointee struct gets a wrapper). Types
/// that are not a single named (possibly-pointed) typedef are left untouched, as are
/// struct/scalar aliases.
fn resolve_pointer_typedef(type_name: &str, raw: &BTreeMap<String, String>) -> String {
    let outer_stars = type_name.matches('*').count();
    let base_part = type_name.replace('*', " ");
    let names: Vec<&str> = base_part
        .split_whitespace()
        .filter(|token| !matches!(*token, "const" | "volatile" | "restrict"))
        .collect();
    if names.len() != 1 {
        return type_name.to_owned();
    }
    let Some((base, extra_stars)) = resolve_pointer_typedef_base(names[0], raw, 0) else {
        return type_name.to_owned();
    };
    // Only reveal the depth of a pointer to an *aggregate* (struct/opaque). A pointer
    // typedef whose base is a byte/scalar (`png_charpp` = `char **`) stays opaque so
    // the existing string/array handling isn't disturbed.
    let core = base
        .strip_prefix("struct ")
        .or_else(|| base.strip_prefix("union "))
        .or_else(|| base.strip_prefix("enum "))
        .unwrap_or(&base);
    if core == "void" || types::scalar_wrapper(core).is_some() {
        return type_name.to_owned();
    }
    let mut out = String::new();
    if type_name.split_whitespace().any(|token| token == "const") {
        out.push_str("const ");
    }
    out.push_str(&base);
    let stars = outer_stars + extra_stars;
    if stars > 0 {
        out.push(' ');
        out.extend(std::iter::repeat_n('*', stars));
    }
    out
}

fn parse_function_signature(line: &str) -> Option<FunctionSignature> {
    let line = line.trim();
    if !line.ends_with(';')
        || !line.contains('(')
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

    let (params, variadic) = parse_params(line[open + 1..close].trim());

    Some(FunctionSignature {
        name,
        return_type,
        params,
        variadic,
        native_symbol: None,
        unsupported: None,
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

fn parse_params(params: &str) -> (Vec<FunctionParam>, bool) {
    if params.trim().is_empty() || params.trim() == "void" {
        return (Vec::new(), false);
    }

    let mut seen = BTreeMap::new();
    let mut variadic = false;
    let params = split_params(params)
        .into_iter()
        .enumerate()
        .filter_map(|(index, param)| {
            if param.trim() == "..." {
                variadic = true;
                None
            } else {
                Some(unique_param_name(parse_param(param, index), &mut seen))
            }
        })
        .collect();

    (params, variadic)
}

fn split_params(params: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                parts.push(params[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(params[start..].trim());
    parts
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
    if let Some(pointer) = parse_function_pointer_param(param, index) {
        return pointer;
    }
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

fn parse_function_pointer_param(param: &str, index: usize) -> Option<FunctionParam> {
    let start = param.find("(*")? + 2;
    let rest = &param[start..];
    let end = rest.find(')')?;
    let name = rest[..end].trim();
    Some(FunctionParam {
        name: sanitize_php_param_name(name, index),
        type_name: "void *".to_owned(),
    })
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
            symbols: &[],
        }
    }

    #[test]
    fn renders_php_methods() {
        let signatures = sample_signatures();
        // Default variant: wrapper returns (scalars_in_return = false).
        insta::assert_snapshot!(render_methods(
            &sample_options(&signatures),
            false,
            false,
            "__pnlx_boot_test_20260615T000000Z"
        ));
    }

    #[test]
    fn out_parameter_param_accepts_wrappers_and_gates_cdata() {
        let signatures = parse_function_signatures("void demo_version(int *major, int *minor);\n");
        let options = sample_options(&signatures);
        let without_cdata = render_methods(&options, false, false, "");
        let with_cdata = render_methods(&options, true, false, "");

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
        insta::assert_snapshot!(render_methods(
            &sample_options(&signatures),
            false,
            true,
            "__pnlx_boot_test_20260615T000000Z"
        ));
    }

    #[test]
    fn renders_php_global_functions() {
        let signatures = sample_signatures();
        insta::assert_snapshot!(render_global_functions(&sample_options(&signatures)));
    }

    #[test]
    fn stamps_boot_token_from_native_hash() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path();
        let entity = generated.join("Demo.php");
        fs::write(
            &entity,
            "<?php\nclass Demo {\n    protected const string PNLX_BOOT_TOKEN = '';\n    public const string PATH = '';\n    public const string HASH = '';\n    public const array LIBRARIES = [];\n}\n",
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
        assert!(stamped.contains("protected const string PNLX_BOOT_TOKEN = '__pnlx_boot_abc123';"));
        assert!(stamped.contains("public const string HASH = 'abc123';"));
        assert!(
            stamped.contains("public const array LIBRARIES = ['/usr/lib/libcblas.so'];"),
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
                type_name: "const char *".to_owned()
            }]
        );
        assert_eq!(signatures[1].name, "qsort");
        assert!(!signatures[1].variadic);
        assert_eq!(
            signatures[1].params[3],
            FunctionParam {
                name: "compar".to_owned(),
                type_name: "void *".to_owned()
            }
        );
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
