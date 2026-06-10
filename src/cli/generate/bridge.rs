use serde::Serialize;
use serde_json::json;

use super::FunctionSignature;
use super::names::bridge_symbol_name;
use super::types::{
    php_bridge_c_type, rust_ffi_return_type, rust_ffi_type, sanitize_php_param_name,
};

/// A C-declaration parameter (`<type> <name>`) for the FFI `cdef` block.
#[derive(Debug, Serialize)]
struct CdefParam {
    c_type: String,
    name: String,
}

/// One native-function prototype in the generated `cdef`.
#[derive(Debug, Serialize)]
struct CdefView {
    return_type: String,
    symbol: String,
    params: Vec<CdefParam>,
}

/// A Rust FFI parameter (`<name>: <type>`).
#[derive(Debug, Serialize)]
struct RustParam {
    name: String,
    rust_type: String,
}

/// One native function for the Rust bridge: its extern declaration and the
/// `#[no_mangle]` wrapper that forwards to it.
#[derive(Debug, Serialize)]
struct BridgeFnView {
    /// The original native symbol called inside the wrapper.
    name: String,
    /// The exported `pnlx_bridge_*` wrapper symbol.
    symbol: String,
    params: Vec<RustParam>,
    return_type: String,
    has_return: bool,
}

pub(super) fn render_bridge_cdef(signatures: &[FunctionSignature]) -> String {
    let functions = signatures
        .iter()
        .map(|signature| CdefView {
            return_type: php_bridge_c_type(&signature.return_type),
            symbol: bridge_symbol_name(signature),
            params: signature
                .params
                .iter()
                .enumerate()
                .map(|(index, param)| CdefParam {
                    c_type: php_bridge_c_type(&param.type_name),
                    name: sanitize_php_param_name(&param.name, index),
                })
                .collect(),
        })
        .collect::<Vec<_>>();

    super::render_inner_template(
        super::BRIDGE_CDEF_TEMPLATE,
        json!({ "functions": functions }),
    )
}

pub(super) fn render_bridge_functions(signatures: &[FunctionSignature]) -> String {
    let functions = signatures
        .iter()
        .map(|signature| {
            let return_type = rust_ffi_return_type(&signature.return_type);
            BridgeFnView {
                name: signature.name.clone(),
                symbol: bridge_symbol_name(signature),
                params: rust_params(signature),
                has_return: return_type != "()",
                return_type,
            }
        })
        .collect::<Vec<_>>();

    super::render_inner_template(
        super::BRIDGE_FUNCTIONS_TEMPLATE,
        json!({ "functions": functions }),
    )
}

fn rust_params(signature: &FunctionSignature) -> Vec<RustParam> {
    signature
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| RustParam {
            name: rust_param_name(&param.name, index),
            rust_type: rust_ffi_type(&param.type_name),
        })
        .collect()
}

fn rust_param_name(name: &str, index: usize) -> String {
    let candidate = sanitize_php_param_name(name, index);
    // Suffix Rust keywords with `_`; this is always a valid identifier, unlike
    // raw identifiers which `crate`/`self`/`super`/`Self` cannot use.
    if is_rust_keyword(&candidate) {
        format!("{candidate}_")
    } else {
        candidate
    }
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "union"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "gen"
    )
}
