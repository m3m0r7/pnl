use super::FunctionSignature;
use super::names::bridge_symbol_name;
use super::types::{
    php_bridge_c_type, rust_ffi_return_type, rust_ffi_type, sanitize_php_param_name,
};

pub(super) fn render_bridge_cdef(signatures: &[FunctionSignature]) -> String {
    let mut out = "typedef unsigned long size_t;\ntypedef signed long ssize_t;\n\n".to_owned();
    for signature in signatures {
        out.push_str(&php_bridge_c_type(&signature.return_type));
        out.push(' ');
        out.push_str(&bridge_symbol_name(signature));
        out.push('(');
        if signature.params.is_empty() {
            out.push_str("void");
        } else {
            out.push_str(
                &signature
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        format!(
                            "{} {}",
                            php_bridge_c_type(&param.type_name),
                            sanitize_php_param_name(&param.name, index)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        out.push_str(");\n");
    }
    out
}

pub(super) fn render_bridge_functions(signatures: &[FunctionSignature]) -> String {
    if signatures.is_empty() {
        return "// No native functions were discovered for this bridge.\n".to_owned();
    }

    let mut out = String::new();
    out.push_str("mod native {\n");
    out.push_str("    use super::*;\n");
    out.push_str("    unsafe extern \"C\" {\n");
    for signature in signatures {
        render_native_declaration(&mut out, signature);
    }
    out.push_str("    }\n");
    out.push_str("}\n\n");
    for signature in signatures {
        render_bridge_wrapper(&mut out, signature);
    }
    out
}

fn render_native_declaration(out: &mut String, signature: &FunctionSignature) {
    out.push_str("        pub fn ");
    out.push_str(&signature.name);
    out.push('(');
    out.push_str(&rust_params(signature));
    out.push(')');
    let return_type = rust_ffi_return_type(&signature.return_type);
    if return_type != "()" {
        out.push_str(" -> ");
        out.push_str(&return_type);
    }
    out.push_str(";\n");
}

fn render_bridge_wrapper(out: &mut String, signature: &FunctionSignature) {
    out.push_str("#[unsafe(no_mangle)]\n");
    out.push_str("pub unsafe extern \"C\" fn ");
    out.push_str(&bridge_symbol_name(signature));
    out.push('(');
    out.push_str(&rust_params(signature));
    out.push(')');
    let return_type = rust_ffi_return_type(&signature.return_type);
    if return_type != "()" {
        out.push_str(" -> ");
        out.push_str(&return_type);
    }
    out.push_str(" {\n");
    out.push_str("    unsafe { native::");
    out.push_str(&signature.name);
    out.push('(');
    out.push_str(&rust_arg_names(signature));
    out.push_str(") }\n");
    out.push_str("}\n\n");
}

fn rust_params(signature: &FunctionSignature) -> String {
    signature
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            format!(
                "{}: {}",
                rust_param_name(&param.name, index),
                rust_ffi_type(&param.type_name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn rust_arg_names(signature: &FunctionSignature) -> String {
    signature
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| rust_param_name(&param.name, index))
        .collect::<Vec<_>>()
        .join(", ")
}

fn rust_param_name(name: &str, index: usize) -> String {
    let candidate = sanitize_php_param_name(name, index);
    match candidate.as_str() {
        "type" | "match" | "ref" | "loop" | "move" | "async" | "await" | "crate" | "self"
        | "super" => format!("r#{candidate}"),
        _ => candidate,
    }
}
