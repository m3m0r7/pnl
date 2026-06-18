pub(super) fn php_ffi_c_type(c_type: &str) -> String {
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
        "int" | "int32_t" | "bool" | "_Bool" | "wchar_t" => "int",
        "unsigned" | "unsigned int" | "uint32_t" | "Uint32" | "wint_t" => "unsigned int",
        "long" | "long int" | "clock_t" => "long",
        "unsigned long" | "unsigned long int" | "wctype_t" | "wctrans_t" => "unsigned long",
        "long long" | "long long int" | "int64_t" | "time_t" | "intmax_t" => "long long",
        "unsigned long long" | "unsigned long long int" | "uint64_t" | "uintmax_t" => {
            "unsigned long long"
        }
        "size_t" => "size_t",
        "ssize_t" => "ssize_t",
        _ => "int",
    }
    .to_owned()
}

/// How a pointer the C function writes through maps onto a by-reference PHP
/// out/in-out parameter.
pub(super) enum PointerOut {
    /// A scalar out (`int *`, `double *`): holds the C element type to allocate.
    Scalar(&'static str),
    /// A C-string out (`char **`): the written `char *` becomes a PHP string.
    StringOut,
    /// A handle out (`T **`): the written `T *` is wrapped in the base context.
    Handle,
}

/// Classify a parameter the native call writes through, for the by-reference
/// out-parameter binding. A single-level pointer to a numeric scalar is a scalar
/// out; a double pointer to `char` is a string out; any other double pointer is a
/// handle out. Everything else (a plain `void *`/struct pointer input, a string
/// input `char *`, deeper pointers) is `None` and keeps its normal handling.
pub(super) fn pointer_out_param(c_type: &str) -> Option<PointerOut> {
    let normalized = normalize_c_type(c_type);
    match normalized.matches('*').count() {
        1 => scalar_pointer_element(&normalized).map(PointerOut::Scalar),
        2 => {
            let last_star = normalized.rfind('*')?;
            let mut pointee = normalized.clone();
            pointee.remove(last_star);
            if is_char_pointer(pointee.trim_end()) {
                Some(PointerOut::StringOut)
            } else {
                Some(PointerOut::Handle)
            }
        }
        _ => None,
    }
}

/// A non-const single-level `char *` (or other byte pointer) parameter: a writable
/// buffer the native call fills in, surfaced as a by-reference out-parameter. A
/// `const char *` is a pure string input and stays by-value; `char **` is a deeper
/// pointer handled by {@see pointer_out_param}. `const` is checked on the raw type
/// because {@see normalize_c_type} strips it.
pub(super) fn writable_char_buffer(raw_type: &str) -> bool {
    let normalized = normalize_c_type(raw_type);
    is_char_pointer(&normalized)
        && normalized.matches('*').count() == 1
        && !raw_type.split_whitespace().any(|token| token == "const")
}

/// PHP namespace holding the generated, self-contained type layer.
pub(super) const HELPERS_NS: &str = "\\Pnlx\\Types";

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
    // Strip a *leading* aggregate keyword only — a substring replace would also
    // chop the `struct` out of a type whose name embeds it (gmp's `__mpz_struct`).
    let stripped = normalized
        .strip_prefix("struct ")
        .or_else(|| normalized.strip_prefix("union "))
        .or_else(|| normalized.strip_prefix("enum "))
        .unwrap_or(&normalized)
        .to_owned();
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
            ffi_type: php_ffi_c_type(&normalized),
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
/// committed `src/sdk/Pnlx/Types/` files against drift from the naming rules.
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
        "clock_t",
        "time_t",
        "wchar_t",
        "wint_t",
        "intmax_t",
        "uintmax_t",
        "wctype_t",
        "wctrans_t",
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
        "long double",
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
        "long double" => "LongDouble",
        "clock_t" => "ClockT",
        "time_t" => "TimeT",
        "wchar_t" => "WcharT",
        "wint_t" => "WintT",
        "intmax_t" => "IntmaxT",
        "uintmax_t" => "UintmaxT",
        "wctype_t" => "WctypeT",
        "wctrans_t" => "WctransT",
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
    // Drop cv/restrict qualifiers as whole tokens, so a *trailing* one — the
    // const-pointer qualifier in `const char * const` (cJSON passes `const char *
    // const string`) — is removed too, not just the leading `const `.
    c_type
        .split_whitespace()
        .filter(|token| {
            !matches!(
                *token,
                "const" | "volatile" | "restrict" | "__restrict" | "__restrict__"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_char_pointer(c_type: &str) -> bool {
    if !is_pointer_type(c_type) {
        return false;
    }
    let base = c_type
        .replace('*', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // A single-level pointer to a byte type is treated as a PHP string: these
    // are how C string/byte-buffer parameters are spelled (`const char *`,
    // `const unsigned char *`, `const uint8_t *`), and the examples pass PHP
    // strings to them. A pointer-to-pointer (`char **`) stays a real pointer.
    if c_type.matches('*').count() != 1 {
        return false;
    }
    matches!(
        base.as_str(),
        "char" | "unsigned char" | "signed char" | "uint8_t" | "int8_t"
    )
}

/// A single-level pointer to `void` (`void *` / `const void *`). PHP FFI accepts a
/// PHP string for such a parameter (it passes the string's bytes as the pointer),
/// so the generated wrapper widens an opaque-pointer parameter of this shape to also
/// admit `string` — the natural input for the byte buffers these spell (libargon2's
/// `const void *pwd`, libssh's `const void *value`, libbz2's `void *buf`). A typed
/// pointer (`struct foo *`) and a pointer-to-pointer (`void **`) are NOT included:
/// FFI rejects a string for those.
pub(super) fn is_void_pointer(c_type: &str) -> bool {
    let normalized = normalize_c_type(c_type);
    if normalized.matches('*').count() != 1 {
        return false;
    }
    normalized
        .replace('*', " ")
        .split_whitespace()
        .filter(|token| !matches!(*token, "const" | "volatile" | "restrict"))
        .collect::<Vec<_>>()
        .join(" ")
        == "void"
}

/// For a single-level pointer to a *numeric scalar* (`int *`, `double *`,
/// `uint32_t *`), the C element type to allocate a holder of, spelled the way the
/// cdef declares it so the FFI type check accepts the holder (`int` and `int32_t`
/// are distinct names to FFI). `None` for char pointers (strings), `void *`,
/// struct/opaque pointers, pointer-to-pointer, and anything FFI's library-less
/// allocator can't size. Used to mark a by-reference out/in-out parameter.
pub(super) fn scalar_pointer_element(c_type: &str) -> Option<&'static str> {
    let normalized = normalize_c_type(c_type);
    if normalized.matches('*').count() != 1 || is_char_pointer(&normalized) {
        return None;
    }
    let base = normalized
        .replace('*', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match base.as_str() {
        "short" | "short int" => Some("short"),
        "unsigned short" | "unsigned short int" => Some("unsigned short"),
        "int" => Some("int"),
        "unsigned" | "unsigned int" => Some("unsigned int"),
        "long" | "long int" => Some("long"),
        "unsigned long" | "unsigned long int" => Some("unsigned long"),
        "long long" | "long long int" => Some("long long"),
        "unsigned long long" | "unsigned long long int" => Some("unsigned long long"),
        "int16_t" => Some("int16_t"),
        "uint16_t" => Some("uint16_t"),
        "int32_t" => Some("int32_t"),
        "uint32_t" => Some("uint32_t"),
        "int64_t" => Some("int64_t"),
        "uint64_t" => Some("uint64_t"),
        "size_t" => Some("size_t"),
        "ssize_t" => Some("ssize_t"),
        "float" => Some("float"),
        "double" => Some("double"),
        _ => None,
    }
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
            | "clock_t"
            | "time_t"
            | "wchar_t"
            | "wint_t"
            | "intmax_t"
            | "uintmax_t"
            | "wctype_t"
            | "wctrans_t"
            | "Sint64"
            | "Uint64"
            | "bool"
            | "_Bool"
            | "SDL_bool"
    )
}

fn is_float_type(c_type: &str) -> bool {
    matches!(c_type, "float" | "double" | "long double")
}

#[cfg(test)]
mod tests {
    use super::{
        is_char_pointer, is_void_pointer, normalize_c_type, scalar_family, scalar_pointer_element,
    };

    #[test]
    fn detects_single_level_void_pointers() {
        // A single-level `void *` accepts a PHP string at the FFI boundary, so the
        // wrapper widens its type. `const`/`volatile` qualifiers do not change that.
        assert!(is_void_pointer("void *"));
        assert!(is_void_pointer("const void *"));
        assert!(is_void_pointer("const void * const"));
        // Not single-level void pointers: typed pointers, pointer-to-pointer, scalars.
        assert!(!is_void_pointer("void **"));
        assert!(!is_void_pointer("struct foo *"));
        assert!(!is_void_pointer("char *"));
        assert!(!is_void_pointer("void"));
        assert!(!is_void_pointer("int"));
    }

    #[test]
    fn detects_scalar_pointer_out_parameters() {
        // A single-level pointer to a numeric scalar is a by-reference out-param,
        // allocated with the cdef's own element spelling.
        assert_eq!(scalar_pointer_element("int *"), Some("int"));
        assert_eq!(
            scalar_pointer_element("unsigned int *"),
            Some("unsigned int")
        );
        assert_eq!(scalar_pointer_element("double *"), Some("double"));
        assert_eq!(scalar_pointer_element("uint32_t *"), Some("uint32_t"));
        assert_eq!(scalar_pointer_element("long *"), Some("long"));
        // Not out-params: strings, void, pointer-to-pointer, struct/opaque pointers.
        assert_eq!(scalar_pointer_element("const char *"), None);
        assert_eq!(scalar_pointer_element("uint8_t *"), None);
        assert_eq!(scalar_pointer_element("void *"), None);
        assert_eq!(scalar_pointer_element("int **"), None);
        assert_eq!(scalar_pointer_element("struct png_struct *"), None);
        assert_eq!(scalar_pointer_element("int"), None);
    }

    #[test]
    fn normalize_strips_trailing_const_pointer_qualifier() {
        // `const char * const` (a const pointer to const char, as cJSON declares
        // its params) must normalise to `char *` so it is recognised as a string.
        assert_eq!(normalize_c_type("const char * const"), "char *");
        assert_eq!(normalize_c_type("const char *"), "char *");
        assert!(is_char_pointer(&normalize_c_type("const char * const")));
        assert!(is_char_pointer(&normalize_c_type("const unsigned char *")));
        assert!(!is_char_pointer(&normalize_c_type("const char **")));
    }

    /// The scalar wrappers ship as committed SDK source (one class per file under
    /// `src/sdk/Pnlx/Types/`), copied into `@pnlx/runtime` at install time. Guard
    /// against drift between the naming rules here and those files: every wrapper
    /// the generator names in return/param types must have a backing file.
    #[test]
    fn scalar_family_matches_committed_sdk_types() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/sdk/Pnlx/Types");
        for wrapper in scalar_family() {
            let path = format!("{dir}/{}.php", wrapper.class);
            let source = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                panic!(
                    "missing SDK helper file for scalar wrapper {}: {path}",
                    wrapper.class,
                )
            });
            // Each wrapper bakes the fundamental C type it allocates a holder/buffer
            // of; keep it in lockstep with the generator's `php_ffi_c_type` mapping.
            let expected = format!("protected const string C_TYPE = '{}';", wrapper.ffi_type);
            assert!(
                source.contains(&expected),
                "{} must declare `{expected}` (got C_TYPE drift)",
                wrapper.class,
            );
        }
    }
}
