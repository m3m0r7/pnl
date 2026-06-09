use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::names::alias_names;
use super::types::{is_char_pointer, normalize_c_type, php_param_type, php_return_type};
use super::{FunctionParam, PhpPackageTemplateOptions};

pub(super) fn render_methods(options: &PhpPackageTemplateOptions<'_>) -> String {
    let prefix = options.function_prefix;
    let mut lines = String::new();
    let mut emitted = BTreeMap::new();

    for signature in options.signatures {
        for name in alias_names(&signature.name) {
            if emitted.insert(name.to_ascii_lowercase(), true).is_some() {
                continue;
            }
            // `--function-prefix` renames the public method but dispatch still
            // uses the original symbol name (the alias map is keyed by it).
            lines.push_str("    public function ");
            lines.push_str(prefix);
            lines.push_str(&name);
            lines.push('(');
            lines.push_str(&php_params(&signature.params));
            lines.push_str("): ");
            lines.push_str(php_return_type(&signature.return_type));
            lines.push_str("\n    {\n");
            lines.push_str(&php_method_body(&signature.return_type, &name));
            lines.push_str("    }\n\n");
        }
    }

    lines
}

pub(super) fn render_global_functions(options: &PhpPackageTemplateOptions<'_>) -> String {
    if options.signatures.is_empty() {
        return String::new();
    }

    let variable = runtime_variable_name(options);
    let prefix = options.function_prefix;
    let mut out = String::new();
    let mut emitted = BTreeSet::new();
    for signature in options.signatures {
        if !emitted.insert(signature.name.clone()) {
            continue;
        }

        // `--function-prefix` renames the function; it dispatches to the matching
        // (also-prefixed) entity method.
        let function_name = format!("{prefix}{}", signature.name);

        // Functions live under `namespace Pnlx\Func\<Class>`, so `function_exists`
        // must be given the fully-qualified name (an unqualified string would test
        // the global namespace instead).
        out.push_str("if (!function_exists('Pnlx\\\\Func\\\\");
        out.push_str(options.class_name);
        out.push_str("\\\\");
        out.push_str(&function_name);
        out.push_str("')) {\n");
        out.push_str("    function ");
        out.push_str(&function_name);
        out.push('(');
        out.push_str(&php_params(&signature.params));
        out.push_str("): ");
        out.push_str(php_return_type(&signature.return_type));
        out.push_str("\n    {\n");
        out.push_str(&php_global_function_body(
            &signature.return_type,
            &variable,
            &function_name,
        ));
        out.push_str("    }\n");
        out.push_str("}\n\n");
    }
    out
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

pub(super) fn php_params(params: &[FunctionParam]) -> String {
    params
        .iter()
        .map(|param| format!("{} ${}", php_param_type(&param.type_name), param.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn php_method_body(c_type: &str, dispatch_name: &str) -> String {
    // Dispatch by the original symbol name (not `__FUNCTION__`, which would be the
    // prefixed method name under `--function-prefix`).
    let call = format!("$this->__call('{dispatch_name}', func_get_args())");
    let c_type = normalize_c_type(c_type);
    if c_type == "void" {
        format!("        {call};\n")
    } else if is_char_pointer(&c_type) {
        format!("        return \\Pnlx\\Util::cString({call});\n")
    } else if php_return_type(&c_type) == "int" {
        format!("        return (int) {call};\n")
    } else if php_return_type(&c_type) == "float" {
        format!("        return (float) {call};\n")
    } else {
        format!("        return {call};\n")
    }
}

fn php_global_function_body(c_type: &str, variable: &str, name: &str) -> String {
    // Emit the literal method name; `__FUNCTION__` would expand to the
    // namespaced `Pnlx\Func\<name>` and would not resolve as a method.
    if normalize_c_type(c_type) == "void" {
        format!("        $GLOBALS['{variable}']->{{'{name}'}}(...func_get_args());\n")
    } else {
        format!("        return $GLOBALS['{variable}']->{{'{name}'}}(...func_get_args());\n")
    }
}
