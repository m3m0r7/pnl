//! Parsing a generated cdef back into typed [`FunctionSignature`]s (with their
//! params, return type, and pointer/enum classification) and the shared
//! [`StructField`]/[`FunctionParam`] types, plus the typedef-resolution helpers
//! that drive PHP type mapping. The inverse of the rendering side in `render.rs`.

use std::collections::{BTreeMap, BTreeSet};

use super::types::{self, sanitize_php_param_name};

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
    /// When the C return type names a generated PHP enum, its enum name. The method
    /// returns `<Enum>|null` via `<Enum>::tryFrom()`; the dispatched C value stays an
    /// `int` (the `return_type` is collapsed to `int`).
    pub(super) return_enum: Option<String>,
}

impl FunctionSignature {
    /// The C symbol the library exports — the rename target when one is set,
    /// otherwise the public name itself.
    pub(super) fn native_symbol(&self) -> &str {
        self.native_symbol.as_deref().unwrap_or(&self.name)
    }
}

/// One field of a generated struct, for emitting a typed accessor on its wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    pub name: String,
    pub type_name: String,
    /// A single-level `wchar_t *` field (detected before the scalar typedef collapses
    /// `wchar_t` to `int`): its accessor decodes a wide string to `?string` via
    /// {@see Util::wcStringOrNull} instead of exposing an `int` pointer.
    pub wide_string: bool,
}

/// Parse the struct definitions the cdef emits with a body (`struct X { int a;
/// char *b; };`) into their field lists, so the generated `Types\X` wrapper can
/// expose typed `getA()/setA()` accessors. Opaque structs (forward-declared only,
/// no body) yield nothing. Field types resolve against the cdef's typedefs exactly
/// like function parameters, and a field whose declarator carries a nested
/// aggregate/function type (`{`/`(`) is skipped (the accessor degrades).
pub fn parse_struct_fields(cdef: &str) -> BTreeMap<String, Vec<StructField>> {
    let raw_typedefs = raw_typedef_map(cdef);
    let scalar_typedefs = scalar_typedef_map(&raw_typedefs);
    let char_pointer_typedefs = char_pointer_typedef_set(&raw_typedefs);
    let mut structs = BTreeMap::new();
    for line in cdef.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("struct ") else {
            continue;
        };
        let Some(brace) = rest.find('{') else {
            continue;
        };
        let tag = rest[..brace].trim();
        if tag.is_empty()
            || !tag
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            continue;
        }
        let Some(close) = rest.rfind('}') else {
            continue;
        };
        let mut fields = Vec::new();
        let mut seen = BTreeSet::new();
        // Split on TOP-LEVEL `;` only: an inline anonymous `union { ... } init`
        // member must be treated as one field (and skipped), not have its own
        // members harvested as struct fields (they would collide, e.g. luaL_Buffer's
        // `lua_State *L` vs the union's `long l` both → `getL`).
        for declaration in split_top_level_members(&rest[brace + 1..close]) {
            let declaration = declaration.trim();
            // A nested aggregate or function-pointer declarator can't be split into a
            // simple name/type pair; skip it (the field stays reachable through cdata()).
            if declaration.is_empty() || declaration.contains('{') || declaration.contains('(') {
                continue;
            }
            let Some((type_name, name)) = split_c_declaration_name(declaration) else {
                continue;
            };
            if !seen.insert(name.clone()) {
                continue;
            }
            // Detect a `wchar_t *` field before the scalar typedef collapses
            // `wchar_t` to `int` (the accessor reads it as a wide string).
            let wide_string = is_wide_char_pointer(&type_name);
            let mut type_name = resolve_scalar_typedef(&type_name, &scalar_typedefs);
            type_name = resolve_char_pointer_typedef(&type_name, &char_pointer_typedefs);
            type_name = resolve_pointer_typedef(&type_name, &raw_typedefs);
            fields.push(StructField {
                name,
                type_name,
                wide_string,
            });
        }
        structs.insert(tag.to_owned(), fields);
    }
    structs
}

/// Whether a field's raw C type is a single-level pointer to `wchar_t` (ignoring
/// `const`), e.g. `wchar_t *` or `const wchar_t *`. A `wchar_t **` is not a string.
fn is_wide_char_pointer(type_name: &str) -> bool {
    let compact: String = type_name
        .replace("const", "")
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    compact == "wchar_t*"
}

/// Split a struct body into its members on top-level `;` only, treating a nested
/// `{ ... }` (an inline anonymous union/struct) as part of the single member it
/// belongs to rather than splitting its inner members out.
fn split_top_level_members(body: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                members.push(&body[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    if start < body.len() {
        members.push(&body[start..]);
    }
    members
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
    /// A C function-pointer parameter (`int (*cb)(int)`). Its `type_name` stays
    /// `void *` so it still marshals as a pointer, but the generated wrapper types
    /// it as a PHP `callable` (PHP FFI accepts a closure for a function-pointer
    /// argument).
    pub(super) callback: bool,
    /// When the parameter's C type names a generated PHP enum, its enum name. The
    /// wrapper accepts `<Enum>|int|…` and marshals the enum's backing `->value`; the
    /// `type_name` is collapsed to `int` so the dispatched value is a plain int.
    pub(super) enum_type: Option<String>,
}

pub fn parse_function_signatures(cdef: &str) -> Vec<FunctionSignature> {
    parse_function_signatures_with_enums(cdef, &BTreeSet::new())
}

/// As [`parse_function_signatures`], but `enum_names` (the generated PHP enums)
/// tags any non-pointer parameter/return whose C type names one of them: its
/// `enum_type`/`return_enum` is recorded and the type collapsed to `int`, so the
/// generated wrapper exposes the PHP enum while the dispatched value stays an int.
pub fn parse_function_signatures_with_enums(
    cdef: &str,
    enum_names: &BTreeSet<String>,
) -> Vec<FunctionSignature> {
    let raw_typedefs = raw_typedef_map(cdef);
    let scalar_typedefs = scalar_typedef_map(&raw_typedefs);
    let char_pointer_typedefs = char_pointer_typedef_set(&raw_typedefs);
    let mut seen = BTreeSet::new();
    cdef.lines()
        .filter_map(parse_function_signature)
        .map(|mut signature| {
            if let Some(name) = enum_value_type(&signature.return_type, enum_names) {
                signature.return_enum = Some(name);
                signature.return_type = "int".to_owned();
            } else {
                signature.return_type =
                    resolve_scalar_typedef(&signature.return_type, &scalar_typedefs);
                signature.return_type =
                    resolve_char_pointer_typedef(&signature.return_type, &char_pointer_typedefs);
                signature.return_type =
                    resolve_pointer_typedef(&signature.return_type, &raw_typedefs);
            }
            for param in &mut signature.params {
                if let Some(name) = enum_value_type(&param.type_name, enum_names) {
                    param.enum_type = Some(name);
                    param.type_name = "int".to_owned();
                    continue;
                }
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

/// If a C type is a by-value reference to one of the generated enums — a single
/// type token (after dropping cv-quals and an `enum` keyword) that names an enum,
/// with no pointer — return that enum's name. A pointer to an enum is not an enum
/// value and keeps its normal pointer handling.
fn enum_value_type(c_type: &str, enum_names: &BTreeSet<String>) -> Option<String> {
    if c_type.contains('*') {
        return None;
    }
    let tokens: Vec<&str> = c_type
        .split_whitespace()
        .filter(|token| !matches!(*token, "const" | "volatile" | "restrict" | "enum"))
        .collect();
    let [name] = tokens.as_slice() else {
        return None;
    };
    enum_names.contains(*name).then(|| (*name).to_owned())
}

/// Parse the cdef's function signatures and append the `unsupported` functions
/// (which are NOT in the cdef, e.g. `static inline`), tagging each so its generated
/// method throws instead of dispatching. The unsupported declarations are parsed
/// alongside the cdef so their parameter/return types resolve against the same
/// typedefs and the stub gets a faithful signature.
pub fn parse_signatures_with_unsupported(
    cdef: &str,
    unsupported: &[crate::native::header_adapter::UnsupportedFunction],
    enum_names: &BTreeSet<String>,
) -> Vec<FunctionSignature> {
    if unsupported.is_empty() {
        return parse_function_signatures_with_enums(cdef, enum_names);
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
    let mut signatures =
        parse_function_signatures_with_enums(&format!("{cdef}\n{declarations}"), enum_names);
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
        // plain function prototype. (A bare `struct foo;`/`struct foo bar;` has no
        // `(`, and a definition has `{}`, so those are already excluded — but a
        // function RETURNING a struct pointer, `struct archive *archive_read_new(…)`,
        // also begins with `struct ` and MUST still parse, so don't reject on that.)
        || line.contains('{')
        || line.contains('}')
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
        return_enum: None,
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
            callback: false,
            enum_type: None,
        };
    }

    FunctionParam {
        name: format!("arg{index}"),
        type_name: param.to_owned(),
        callback: false,
        enum_type: None,
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
        callback: true,
        enum_type: None,
    })
}
