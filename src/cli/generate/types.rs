pub(super) fn php_bridge_c_type(c_type: &str) -> String {
    let c_type = normalize_c_type(c_type);
    if c_type == "void" {
        return "void".to_owned();
    }
    if is_char_pointer(&c_type) {
        return "const char *".to_owned();
    }
    if is_pointer_type(&c_type) {
        return "void *".to_owned();
    }
    if is_float_type(&c_type) {
        return c_type;
    }

    match c_type
        .replace('*', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .as_str()
    {
        "char" | "signed char" => "signed char",
        "unsigned char" | "uint8_t" | "Uint8" => "unsigned char",
        "short" | "short int" | "int16_t" => "short",
        "unsigned short" | "unsigned short int" | "uint16_t" | "Uint16" => "unsigned short",
        "int" | "int32_t" | "bool" | "_Bool" => "int",
        "unsigned" | "unsigned int" | "uint32_t" | "Uint32" => "unsigned int",
        "long" | "long int" => "long",
        "unsigned long" | "unsigned long int" => "unsigned long",
        "long long" | "long long int" | "int64_t" => "long long",
        "unsigned long long" | "unsigned long long int" | "uint64_t" => "unsigned long long",
        "size_t" => "size_t",
        "ssize_t" => "ssize_t",
        _ => "int",
    }
    .to_owned()
}

pub(super) fn rust_ffi_return_type(c_type: &str) -> String {
    let c_type = normalize_c_type(c_type);
    if c_type == "void" {
        "()".to_owned()
    } else {
        rust_ffi_type(&c_type)
    }
}

pub(super) fn rust_ffi_type(c_type: &str) -> String {
    let c_type = normalize_c_type(c_type);
    if c_type.contains('*') || c_type.ends_with("[]") {
        let pointer = if c_type.starts_with("const ") || c_type.contains("const ") {
            "*const"
        } else {
            "*mut"
        };
        let pointee = c_type
            .replace("const ", "")
            .replace("volatile ", "")
            .replace('*', " ")
            .replace("[]", " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if pointee == "char" {
            return format!("{pointer} c_char");
        }
        return format!("{pointer} c_void");
    }

    match c_type.as_str() {
        "char" => "c_char",
        "signed char" => "c_schar",
        "unsigned char" | "uint8_t" | "Uint8" => "c_uchar",
        "short" | "short int" | "int16_t" => "c_short",
        "unsigned short" | "unsigned short int" | "uint16_t" | "Uint16" => "c_ushort",
        "int" | "int32_t" | "bool" | "_Bool" => "c_int",
        "unsigned" | "unsigned int" | "uint32_t" | "Uint32" => "c_uint",
        "long" | "long int" => "c_long",
        "unsigned long" | "unsigned long int" => "c_ulong",
        "long long" | "long long int" | "int64_t" => "c_longlong",
        "unsigned long long" | "unsigned long long int" | "uint64_t" => "c_ulonglong",
        "size_t" => "usize",
        "ssize_t" => "isize",
        "float" => "c_float",
        "double" => "c_double",
        _ => "c_int",
    }
    .to_owned()
}

/// PHP namespace holding the generated, self-contained type layer.
pub(super) const HELPERS_NS: &str = "\\Pnlx\\Helpers";

/// A generated scalar wrapper class: its PHP name, the primitive C type it
/// corresponds to, whether it is unsigned (so a returned value that overflowed
/// PHP's signed int is rendered back as unsigned), and whether it is a
/// floating-point type. `toValue()` hands the native call a PHP scalar.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct ScalarWrapper {
    pub class: String,
    pub ffi_type: String,
    pub unsigned: bool,
    pub is_float: bool,
    /// Whether a PHP signed-64 int / float represents this losslessly. False only
    /// for 64-bit unsigned integers (computed from the original C token, not the
    /// possibly-collapsed `ffi_type`).
    pub fits_native: bool,
}

/// How a C type maps onto the generated PHP value layer.
pub(super) enum ValueKind {
    Void,
    /// `char *` → the generated `String_` wrapper.
    Str,
    Int(ScalarWrapper),
    Float(ScalarWrapper),
    /// A pointer/handle. `Some(class)` is the per-type class name (pointee or
    /// typedef); `None` means an opaque pointer (`void *`) that falls back to the
    /// package's base context wrapper.
    Pointer(Option<String>),
}

pub(super) fn value_kind(c_type: &str) -> ValueKind {
    let n = normalize_c_type(c_type);
    if n == "void" {
        return ValueKind::Void;
    }
    if is_char_pointer(&n) {
        return ValueKind::Str;
    }
    if !is_pointer_type(&n)
        && !is_struct_type(&n)
        && let Some(wrapper) = scalar_wrapper(&n)
    {
        return if wrapper.is_float {
            ValueKind::Float(wrapper)
        } else {
            ValueKind::Int(wrapper)
        };
    }
    ValueKind::Pointer(pointer_type_name(&n))
}

/// The per-type pointer wrapper class name (pointee identifier or typedef), or
/// `None` for `void *`, pointer-to-pointer, and other un-nameable pointers.
pub(super) fn pointer_type_name(c_type: &str) -> Option<String> {
    let normalized = normalize_c_type(c_type);
    if is_char_pointer(&normalized) {
        return None;
    }
    let stripped = normalized
        .replace("struct ", "")
        .replace("union ", "")
        .replace("enum ", "");
    if stripped.matches('*').count() > 1 {
        return None;
    }
    let base = stripped
        .replace('*', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if base.is_empty() || base == "void" || base.split_whitespace().count() != 1 {
        return None;
    }
    if !base
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || base.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(reserved_suffix(&base))
}

/// The scalar wrapper for an integer/float C type, or `None` for pointers and
/// unrecognized types.
pub(super) fn scalar_wrapper(c_type: &str) -> Option<ScalarWrapper> {
    let normalized = normalize_c_type(c_type);
    if normalized.contains('*') {
        return None;
    }
    if is_float_type(&normalized) {
        return Some(ScalarWrapper {
            class: reserved_suffix(&pascal_type_name(&normalized)?),
            ffi_type: normalized.clone(),
            unsigned: false,
            is_float: true,
            fits_native: true,
        });
    }
    if is_integer_type(&normalized) {
        return Some(ScalarWrapper {
            class: reserved_suffix(&pascal_type_name(&normalized)?),
            ffi_type: php_bridge_c_type(&normalized),
            unsigned: is_unsigned_integer(&normalized),
            is_float: false,
            fits_native: !is_unsigned_64bit(&normalized),
        });
    }
    None
}

/// Whether the C type is a 64-bit unsigned integer (cannot fit PHP's signed int).
/// `unsigned long` is treated as 64-bit (the LP64 case) to stay lossless.
fn is_unsigned_64bit(c_type: &str) -> bool {
    matches!(
        normalize_c_type(c_type).as_str(),
        "unsigned long"
            | "unsigned long int"
            | "unsigned long long"
            | "unsigned long long int"
            | "uint64_t"
            | "size_t"
            | "Uint64"
    )
}

/// Every scalar wrapper in the SDK type layer, deduplicated by class name
/// (synonyms like `short int`/`short` collapse). Used by tests to guard the
/// committed `src/sdk/Pnlx/Helpers/` files against drift from the naming rules.
#[cfg(test)]
pub(super) fn scalar_family() -> Vec<ScalarWrapper> {
    const TOKENS: &[&str] = &[
        "char",
        "signed char",
        "unsigned char",
        "short",
        "unsigned short",
        "int",
        "unsigned int",
        "long",
        "unsigned long",
        "long long",
        "unsigned long long",
        "int8_t",
        "uint8_t",
        "int16_t",
        "uint16_t",
        "int32_t",
        "uint32_t",
        "int64_t",
        "uint64_t",
        "size_t",
        "ssize_t",
        "bool",
        "_Bool",
        "SDL_bool",
        "Sint8",
        "Uint8",
        "Sint16",
        "Uint16",
        "Sint32",
        "Uint32",
        "Sint64",
        "Uint64",
        "float",
        "double",
    ];
    let mut seen = std::collections::BTreeSet::new();
    let mut family = Vec::new();
    for token in TOKENS {
        if let Some(wrapper) = scalar_wrapper(token)
            && seen.insert(wrapper.class.clone())
        {
            family.push(wrapper);
        }
    }
    family
}

/// Whether this scalar can be represented losslessly by a PHP native scalar
/// (signed 64-bit int / float). Only 64-bit unsigned integers cannot, so under
/// `features.use_php_scalars_in_return` everything else returns a plain
/// `int`/`float` and only these stay wrapped.
pub(super) fn fits_php_scalar(wrapper: &ScalarWrapper) -> bool {
    wrapper.fits_native
}

fn is_unsigned_integer(c_type: &str) -> bool {
    let t = normalize_c_type(c_type);
    t.starts_with("unsigned") || t.starts_with("uint") || t.starts_with("Uint") || t == "size_t"
}

/// PascalCase a known scalar C type token per the naming rules
/// (`unsigned long`→`UnsignedLong`, `uint32_t`→`UnsignedInt32T`, `Uint32`→`UnsignedInt32`,
/// `size_t`→`SizeT`, `long`→`Long`, …). Returns `None` for unknown tokens.
fn pascal_type_name(token: &str) -> Option<String> {
    let name = match normalize_c_type(token).as_str() {
        "char" => "Char",
        "signed char" => "SignedChar",
        "unsigned char" => "UnsignedChar",
        "short" | "short int" => "Short",
        "unsigned short" | "unsigned short int" => "UnsignedShort",
        "int" => "Int",
        "unsigned" | "unsigned int" => "UnsignedInt",
        "long" | "long int" => "Long",
        "unsigned long" | "unsigned long int" => "UnsignedLong",
        "long long" | "long long int" => "LongLong",
        "unsigned long long" | "unsigned long long int" => "UnsignedLongLong",
        "int8_t" => "Int8T",
        "uint8_t" => "UnsignedInt8T",
        "int16_t" => "Int16T",
        "uint16_t" => "UnsignedInt16T",
        "int32_t" => "Int32T",
        "uint32_t" => "UnsignedInt32T",
        "int64_t" => "Int64T",
        "uint64_t" => "UnsignedInt64T",
        "size_t" => "SizeT",
        "ssize_t" => "SsizeT",
        "bool" | "_Bool" => "Bool",
        "SDL_bool" => "SdlBool",
        "Sint8" => "SignedInt8",
        "Uint8" => "UnsignedInt8",
        "Sint16" => "SignedInt16",
        "Uint16" => "UnsignedInt16",
        "Sint32" => "SignedInt32",
        "Uint32" => "UnsignedInt32",
        "Sint64" => "SignedInt64",
        "Uint64" => "UnsignedInt64",
        "float" => "Float",
        "double" => "Double",
        _ => return None,
    };
    Some(name.to_owned())
}

/// Append `_` when a derived class name collides with a PHP reserved type word
/// (case-insensitive), e.g. `Int`→`Int_`, `Float`→`Float_`, `String`→`String_`.
fn reserved_suffix(name: &str) -> String {
    const RESERVED: &[&str] = &[
        "int", "float", "bool", "string", "void", "iterable", "object", "mixed", "never", "null",
        "false", "true", "parent", "self", "static", "enum", "list", "callable", "array",
    ];
    if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

pub(super) fn normalize_c_type(c_type: &str) -> String {
    c_type
        .replace("const ", "")
        .replace("volatile ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_char_pointer(c_type: &str) -> bool {
    if !is_pointer_type(c_type) {
        return false;
    }
    c_type
        .replace('*', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        == "char"
}

pub(super) fn sanitize_php_param_name(name: &str, index: usize) -> String {
    let name = name.trim_start_matches('*').trim_end_matches("[]");
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || name.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    {
        format!("arg{index}")
    } else {
        name.to_owned()
    }
}

fn is_pointer_type(c_type: &str) -> bool {
    c_type.contains('*')
}

fn is_struct_type(c_type: &str) -> bool {
    c_type.starts_with("struct ")
}

fn is_integer_type(c_type: &str) -> bool {
    let type_name = c_type.replace('*', " ");
    let type_name = type_name.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        type_name.as_str(),
        "char"
            | "signed char"
            | "unsigned char"
            | "short"
            | "short int"
            | "unsigned short"
            | "unsigned short int"
            | "int"
            | "unsigned"
            | "unsigned int"
            | "long"
            | "long int"
            | "unsigned long"
            | "unsigned long int"
            | "long long"
            | "long long int"
            | "unsigned long long"
            | "unsigned long long int"
            | "size_t"
            | "ssize_t"
            | "int8_t"
            | "uint8_t"
            | "Sint8"
            | "Uint8"
            | "int16_t"
            | "uint16_t"
            | "Sint16"
            | "Uint16"
            | "int32_t"
            | "uint32_t"
            | "Sint32"
            | "Uint32"
            | "int64_t"
            | "uint64_t"
            | "Sint64"
            | "Uint64"
            | "bool"
            | "_Bool"
            | "SDL_bool"
    )
}

fn is_float_type(c_type: &str) -> bool {
    matches!(c_type, "float" | "double")
}

#[cfg(test)]
mod tests {
    use super::scalar_family;

    /// The scalar wrappers ship as committed SDK source (one class per file under
    /// `src/sdk/Pnlx/Helpers/`), copied into `@pnlx/runtime` at install time. Guard
    /// against drift between the naming rules here and those files: every wrapper
    /// the generator names in return/param types must have a backing file.
    #[test]
    fn scalar_family_matches_committed_sdk_helpers() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/sdk/Pnlx/Helpers");
        for wrapper in scalar_family() {
            let path = format!("{dir}/{}.php", wrapper.class);
            assert!(
                std::path::Path::new(&path).exists(),
                "missing SDK helper file for scalar wrapper {}: {path}",
                wrapper.class,
            );
        }
    }
}
