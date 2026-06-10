use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::names::alias_names;
use super::types::{is_char_pointer, normalize_c_type, php_param_type, php_return_type};
use super::{FunctionParam, PhpPackageTemplateOptions};

/// A PHP parameter as the templates consume it: a type hint and a name.
#[derive(Debug, Serialize)]
struct PhpParam {
    php_type: String,
    name: String,
}

/// A generated entity method, with everything the template needs to emit it.
#[derive(Debug, Serialize)]
struct MethodView {
    /// The public method name (with `--function-prefix` applied).
    name: String,
    /// The original symbol name dispatched through `__call`.
    dispatch: String,
    params: Vec<PhpParam>,
    return_type: String,
    is_void: bool,
    /// Wrap the result with `\Pnlx\Util::cString` (a `char *` return).
    cstring: bool,
    /// A leading cast for scalar returns: `(int) `, `(float) `, or empty.
    cast: String,
}

/// A generated global helper function.
#[derive(Debug, Serialize)]
struct FunctionView {
    name: String,
    /// The fully-qualified `function_exists` name, built in Rust because its
    /// backslashes must not sit next to a `{{ }}` placeholder (Handlebars treats
    /// `\{{` as an escape and would drop a backslash).
    fqn: String,
    params: Vec<PhpParam>,
    return_type: String,
    is_void: bool,
}

pub(super) fn render_methods(options: &PhpPackageTemplateOptions<'_>) -> String {
    let prefix = options.function_prefix;
    let mut methods = Vec::new();
    let mut emitted = BTreeMap::new();

    for signature in options.signatures {
        for name in alias_names(&signature.name) {
            if emitted.insert(name.to_ascii_lowercase(), true).is_some() {
                continue;
            }
            // `--function-prefix` renames the public method but dispatch still
            // uses the original symbol name (the alias map is keyed by it).
            methods.push(method_view(prefix, &name, signature));
        }
    }

    super::render_inner_template(super::METHODS_TEMPLATE, json!({ "methods": methods }))
}

pub(super) fn render_global_functions(options: &PhpPackageTemplateOptions<'_>) -> String {
    if options.signatures.is_empty() {
        return String::new();
    }

    let prefix = options.function_prefix;
    let mut functions = Vec::new();
    let mut emitted = BTreeSet::new();
    for signature in options.signatures {
        if !emitted.insert(signature.name.clone()) {
            continue;
        }
        // `--function-prefix` renames the function; it dispatches to the matching
        // (also-prefixed) entity method.
        let c_type = normalize_c_type(&signature.return_type);
        let name = format!("{prefix}{}", signature.name);
        functions.push(FunctionView {
            // `\\` segments: a PHP single-quoted literal of `Pnlx\Func\<Class>\<name>`.
            fqn: format!("Pnlx\\\\Func\\\\{}\\\\{name}", options.class_name),
            name,
            params: php_params(&signature.params),
            return_type: php_return_type(&signature.return_type).to_owned(),
            is_void: c_type == "void",
        });
    }

    // `runtime_var` is referenced as `../` from inside the each-loop (the
    // `$GLOBALS` runtime key shared by every function).
    super::render_inner_template(
        super::GLOBAL_FUNCTIONS_TEMPLATE,
        json!({
            "runtime_var": runtime_variable_name(options),
            "functions": functions,
        }),
    )
}

fn method_view(prefix: &str, name: &str, signature: &super::FunctionSignature) -> MethodView {
    let c_type = normalize_c_type(&signature.return_type);
    let is_void = c_type == "void";
    let cstring = !is_void && is_char_pointer(&c_type);
    let cast = if is_void || cstring {
        String::new()
    } else {
        match php_return_type(&c_type) {
            "int" => "(int) ".to_owned(),
            "float" => "(float) ".to_owned(),
            _ => String::new(),
        }
    };

    MethodView {
        name: format!("{prefix}{name}"),
        dispatch: name.to_owned(),
        params: php_params(&signature.params),
        return_type: php_return_type(&signature.return_type).to_owned(),
        is_void,
        cstring,
        cast,
    }
}

pub(super) fn runtime_variable_name(options: &PhpPackageTemplateOptions<'_>) -> String {
    let digest = Sha256::digest(
        format!(
            "{}\\{}:{}",
            options.namespace, options.class_name, options.library_key
        )
        .as_bytes(),
    );
    format!("runtime_{digest:x}")
}

fn php_params(params: &[FunctionParam]) -> Vec<PhpParam> {
    params
        .iter()
        .map(|param| PhpParam {
            php_type: php_param_type(&param.type_name).to_owned(),
            name: param.name.clone(),
        })
        .collect()
}
