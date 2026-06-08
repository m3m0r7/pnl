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

pub(super) fn php_return_type(c_type: &str) -> &'static str {
    let c_type = normalize_c_type(c_type);
    if c_type == "void" {
        "void"
    } else if is_char_pointer(&c_type) {
        "string"
    } else if is_integer_type(&c_type) {
        "int"
    } else if is_float_type(&c_type) {
        "float"
    } else {
        "\\FFI\\CData"
    }
}

pub(super) fn php_param_type(c_type: &str) -> &'static str {
    let c_type = normalize_c_type(c_type);
    if is_char_pointer(&c_type) {
        "string|\\FFI\\CData|null"
    } else if is_pointer_type(&c_type) || is_struct_type(&c_type) {
        "\\FFI\\CData|null"
    } else if is_integer_type(&c_type) {
        "int"
    } else if is_float_type(&c_type) {
        "float"
    } else {
        "\\FFI\\CData"
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
