use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::names::alias_names;
use super::types::{is_char_pointer, normalize_c_type, php_param_type, php_return_type};
use super::{FunctionParam, FunctionSignature, PhpPackageTemplateOptions};

pub(super) fn render_methods(signatures: &[FunctionSignature]) -> String {
    let mut lines = String::new();
    let mut emitted = BTreeMap::new();

    for signature in signatures {
        for name in alias_names(&signature.name) {
            if emitted.insert(name.to_ascii_lowercase(), true).is_some() {
                continue;
            }
            lines.push_str("    public function ");
            lines.push_str(&name);
            lines.push('(');
            lines.push_str(&php_params(&signature.params));
            lines.push_str("): ");
            lines.push_str(php_return_type(&signature.return_type));
            lines.push_str("\n    {\n");
            lines.push_str(&php_method_body(&signature.return_type));
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
    let mut out = String::new();
    let mut emitted = BTreeSet::new();
    for signature in options.signatures {
        if !emitted.insert(signature.name.clone()) {
            continue;
        }

        // Functions live under `namespace Pnlx\Func\<Class>`, so `function_exists`
        // must be given the fully-qualified name (an unqualified string would test
        // the global namespace instead).
        out.push_str("if (!function_exists('Pnlx\\\\Func\\\\");
        out.push_str(options.class_name);
        out.push_str("\\\\");
        out.push_str(&signature.name);
        out.push_str("')) {\n");
        out.push_str("    function ");
        out.push_str(&signature.name);
        out.push('(');
        out.push_str(&php_params(&signature.params));
        out.push_str("): ");
        out.push_str(php_return_type(&signature.return_type));
        out.push_str("\n    {\n");
        out.push_str(&php_global_function_body(
            &signature.return_type,
            &variable,
            &signature.name,
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

fn php_method_body(c_type: &str) -> String {
    let c_type = normalize_c_type(c_type);
    if c_type == "void" {
        "        $this->__call(__FUNCTION__, func_get_args());\n".to_owned()
    } else if is_char_pointer(&c_type) {
        "        return \\Pnlx\\Util::cString($this->__call(__FUNCTION__, func_get_args()));\n"
            .to_owned()
    } else if php_return_type(&c_type) == "int" {
        "        return (int) $this->__call(__FUNCTION__, func_get_args());\n".to_owned()
    } else if php_return_type(&c_type) == "float" {
        "        return (float) $this->__call(__FUNCTION__, func_get_args());\n".to_owned()
    } else {
        "        return $this->__call(__FUNCTION__, func_get_args());\n".to_owned()
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
