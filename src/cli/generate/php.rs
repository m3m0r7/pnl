use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::json;

use super::names::{method_names, snake_to_pascal};
use super::types::{
    HELPERS_NS, PointerOut, ValueKind, fits_php_scalar, is_void_pointer, pointer_out_param,
    reserved_suffix, value_kind, writable_char_buffer,
};
use super::{FunctionParam, FunctionSignature, PhpPackageTemplateOptions, StructField};

/// The PHP class name for a C enum: the C tag, suffixed if it collides with a PHP
/// reserved word so it is a legal class name (`enum`→`enum_`).
pub(super) fn enum_class_name(c_name: &str) -> String {
    reserved_suffix(c_name)
}

/// One struct field rendered as a typed accessor pair on the `Types\<tag>` wrapper.
/// Exactly one type flag is set; with none, a `mixed` accessor reads/writes the raw
/// `\FFI\CData` field (so a field whose type the value layer can't name is still
/// reachable). A `char *` field gets a read-only `?string` getter (writing a string
/// field needs C-side allocation/lifetime, deferred).
#[derive(Debug, Serialize)]
struct FieldView {
    field: String,
    getter: String,
    setter: String,
    is_int: bool,
    is_float: bool,
    is_string: bool,
    /// A `wchar_t *` field: the getter decodes a wide string to `?string`.
    is_wide_string: bool,
    pointer_class: Option<String>,
}

/// The accessor views for a struct's fields, as JSON for the `Types\<tag>` template.
pub(super) fn struct_field_views(
    fields: &[StructField],
    options: &PhpPackageTemplateOptions<'_>,
) -> Vec<serde_json::Value> {
    // PHP method names are case-insensitive, so two fields whose accessors collide
    // (e.g. a `Foo` and a `foo` field, both → `getFoo`) would redeclare. Keep the
    // first; a later collider stays reachable through `cdata()`.
    let mut taken_accessors = BTreeSet::new();
    fields
        .iter()
        .filter_map(|field| {
            let pascal = snake_to_pascal(&field.name);
            if !taken_accessors.insert(pascal.to_ascii_lowercase()) {
                return None;
            }
            let mut view = FieldView {
                field: field.name.clone(),
                getter: format!("get{pascal}"),
                setter: format!("set{pascal}"),
                is_int: false,
                is_float: false,
                is_string: false,
                is_wide_string: false,
                pointer_class: None,
            };
            // A `wchar_t *` field reads as a wide string; the scalar typedef has
            // already collapsed its type to an `int` pointer, so use the flag.
            if field.wide_string {
                view.is_wide_string = true;
            } else {
                match value_kind(&field.type_name) {
                    ValueKind::Int(_) => view.is_int = true,
                    ValueKind::Float(_) => view.is_float = true,
                    ValueKind::Str => view.is_string = true,
                    ValueKind::Pointer(Some(class)) => {
                        view.pointer_class = Some(format!("{}\\{class}", types_ns(options)));
                    }
                    ValueKind::Pointer(None) | ValueKind::Void => {}
                }
            }
            Some(json!(view))
        })
        .collect()
}

/// One native-dispatch argument: the template marshals it with `unwrap` (a
/// pointer) or `scalarArg` (a scalar), or — for a scalar-pointer out-parameter —
/// passes it by reference for `OutParameterMarshaller` to handle.
#[derive(Debug, Serialize)]
struct ArgView {
    name: String,
    is_pointer: bool,
    /// A by-reference out/in-out scalar-pointer arg (`int *`, …).
    is_native_pointer: bool,
}

/// One method/function parameter. The template builds the accepted type union
/// from these flags (exactly one type flag set); `cdata` widens it with
/// `\FFI\CData`, and `pointer_class` is the package-specific wrapper FQN.
/// `native_pointer` (the C element type) marks a by-reference out/in-out param.
#[derive(Debug, Serialize)]
struct ParamView {
    name: String,
    is_string: bool,
    is_int: bool,
    is_float: bool,
    is_pointer: bool,
    pointer_class: Option<String>,
    /// A single-level `void *` parameter, whose accepted-type union also admits a
    /// PHP `string` (PHP FFI passes the string's bytes as the pointer).
    void_pointer: bool,
    /// A C function-pointer parameter: the wrapper types it as a PHP `callable` (a
    /// closure/function name PHP FFI turns into a C callback). Mutually exclusive
    /// with the other type flags.
    callback: bool,
    /// When the parameter's C type is a generated enum, its FQCN — added to the
    /// accepted-type union alongside `int` (the enum's backing value is marshalled).
    enum_class: Option<String>,
    cdata: bool,
    /// Whether this is a by-reference out/in-out pointer parameter (`int *`,
    /// `char **`, `T **`), rendered with `#[NativePointer(...)]` and `&$name`.
    /// The `np_*` fields carry the holder element and the write-back mode. The
    /// accepted-type union still comes from the `is_*` flags set alongside.
    native_pointer: bool,
    np_element: String,
    np_string: bool,
    /// A writable single-level `char *` byte buffer (non-const): the caller passes a
    /// pre-sized string, the marshaller copies it into a `char[len]` and writes all
    /// `len` bytes back. Mutually exclusive with `np_string`/`np_wrap`.
    np_buffer: bool,
    np_wrap: Option<String>,
    /// Whether to render `= null` so the by-reference out-parameter can be omitted
    /// (the PHP-expressible equivalent of passing C `NULL` — a literal can't go to
    /// a by-ref slot). Only set on a trailing run of out-parameters, where PHP
    /// allows the default.
    default_null: bool,
}

/// A generated entity method. The template builds the body from these fields;
/// exactly one of the `is_*` return-kind flags is set.
#[derive(Debug, Serialize)]
struct MethodView {
    /// The public method name (with `--function-prefix` applied).
    name: String,
    /// The original C-library symbol name (without `--function-prefix` or the
    /// alias-case variation) — emitted as the `#[RawNativeName(...)]` attribute.
    raw_name: String,
    /// The parameters, whose accepted-type unions the template renders.
    params: Vec<ParamView>,
    /// The C symbol dispatched through `static::__callStatic('...', [...])`.
    call_name: String,
    /// The marshalled arguments passed to the dispatch.
    args: Vec<ArgView>,
    /// Whether the C prototype ends in `...`; direct FFI forwards these raw.
    variadic: bool,
    /// `void`: no return, just dispatch.
    is_void: bool,
    /// A `string` return: the native cdef value is a C string.
    is_string: bool,
    /// Whether the value comes back wrapped (`String_`) or native (the `scalar/`
    /// variant); only meaningful with `is_string`.
    native_string: bool,
    /// A native `(int)`/`(float)` cast return (the `scalar/` variant).
    cast: Option<&'static str>,
    /// A `return new <new_class>(...)` return (wrapped scalar or pointer wrapper).
    new_class: Option<String>,
    /// When the C return is a generated enum, its FQCN: the method returns
    /// `<Enum>|null` via `<Enum>::tryFrom()` (null for a value outside the enum).
    /// Takes precedence over the scalar/pointer return forms.
    enum_return: Option<String>,
    /// Whether any parameter is a scalar-pointer out/in-out param, so the call is
    /// routed through `OutParameterMarshaller::call` (allocate holder, write back).
    has_out_params: bool,
    /// When set, the method has no FFI binding (e.g. a `static inline` with no
    /// exported symbol): its body throws with this reason and it carries the
    /// matching marker attribute, instead of dispatching.
    unsupported_reason: Option<String>,
    /// The PHP attribute marking why the method is unsupported (e.g. `StaticInline`),
    /// derived from `unsupported_reason`.
    unsupported_attribute: Option<String>,
}

/// A generated global helper function. It dispatches to whichever entity variant
/// is loaded, so a fits-native scalar return is declared as the union of the
/// native type and the wrapper (`return_native` + `return_class`).
#[derive(Debug, Serialize)]
struct FunctionView {
    name: String,
    raw_name: String,
    /// The fully-qualified `function_exists` name, built in Rust because its
    /// backslashes must not sit next to a `{{ }}` placeholder (Handlebars treats
    /// `\{{` as an escape and would drop a backslash).
    fqn: String,
    params: Vec<ParamView>,
    variadic: bool,
    is_void: bool,
    /// The native half of the return union (`int`/`float`/`string`), if any.
    return_native: Option<&'static str>,
    /// The non-native return type (wrapper or pointer FQN), if not void.
    return_class: Option<String>,
    /// Whether any parameter is a scalar-pointer out/in-out param, so the wrapper
    /// must declare by-reference params and forward them by name (not func_get_args,
    /// which drops references).
    has_out_params: bool,
    /// Whether the return can be `null` (a pointer that the native call may not
    /// produce), so the forwarding wrapper's return type admits it.
    nullable_return: bool,
    /// When the C return is a generated enum, its FQCN: the function returns
    /// `<Enum>|null` (the entity method maps it via `<Enum>::tryFrom()`), taking
    /// precedence over the scalar/pointer return forms.
    enum_return: Option<String>,
}

/// FQ namespace of this package's per-type pointer wrapper classes.
fn types_ns(options: &PhpPackageTemplateOptions<'_>) -> String {
    format!("\\{}\\Types", options.namespace)
}

/// FQ name of this package's base context wrapper (opaque pointer fallback).
fn base_context(options: &PhpPackageTemplateOptions<'_>) -> String {
    format!("\\{}\\{}Context", options.namespace, options.class_name)
}

/// Render the entity method bodies for one entity variant. `allow_cdata` widens
/// pointer/scalar params to also accept a raw `\FFI\CData`; `scalars_in_return`
/// returns PHP-native scalars (the `scalar/` variant) instead of wrappers.
pub(super) fn render_methods(
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
    scalars_in_return: bool,
) -> String {
    let prefix = options.function_prefix;
    let mut methods = Vec::new();
    let mut emitted = BTreeMap::new();

    for signature in options.signatures {
        for name in method_names(&signature.name) {
            if emitted.insert(name.to_ascii_lowercase(), true).is_some() {
                continue;
            }
            // `--function-prefix` renames the public method but dispatch still
            // uses the original symbol name (the alias map is keyed by it).
            methods.push(method_view(
                prefix,
                &name,
                signature,
                options,
                allow_cdata,
                scalars_in_return,
            ));
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
        // An unsupported (`static inline`) function has no FFI binding to delegate
        // to; its throwing stub lives only as an entity method, not a global function.
        if signature.unsupported.is_some() {
            continue;
        }
        if !emitted.insert(signature.name.clone()) {
            continue;
        }
        // `--function-prefix` renames the function; it dispatches to the matching
        // (also-prefixed) entity method. Global functions accept the permissive
        // (cdata-allowing) parameter union so they work with either entity variant.
        let name = format!("{prefix}{}", signature.name);
        if is_php_reserved_function_name(&name) {
            continue;
        }
        let kind = value_kind(&signature.return_type);
        let (return_native, return_class) = match &kind {
            ValueKind::Void => (None, None),
            ValueKind::Str => (Some("string"), Some(format!("{HELPERS_NS}\\String_"))),
            ValueKind::Int(wrapper) if fits_php_scalar(wrapper) => (
                Some("int"),
                Some(format!("{HELPERS_NS}\\{}", wrapper.class)),
            ),
            ValueKind::Float(wrapper) if fits_php_scalar(wrapper) => (
                Some("float"),
                Some(format!("{HELPERS_NS}\\{}", wrapper.class)),
            ),
            ValueKind::Int(wrapper) | ValueKind::Float(wrapper) => {
                (None, Some(format!("{HELPERS_NS}\\{}", wrapper.class)))
            }
            ValueKind::Pointer(Some(class)) => {
                (None, Some(format!("{}\\{class}", types_ns(options))))
            }
            ValueKind::Pointer(None) => (None, Some(base_context(options))),
        };
        let params = param_views(&signature.params, options, true);
        functions.push(FunctionView {
            // `\\` segments: a PHP single-quoted literal of `Pnlx\Func\<Class>\<name>`.
            fqn: format!("Pnlx\\\\Func\\\\{}\\\\{name}", options.class_name),
            name,
            raw_name: signature.native_symbol().to_owned(),
            has_out_params: params.iter().any(|param| param.native_pointer),
            // A `char *` return is nullable too: a NULL pointer becomes PHP null
            // (see Util::cStringOrNull), so the union must admit null.
            nullable_return: matches!(kind, ValueKind::Pointer(_) | ValueKind::Str),
            params,
            variadic: signature.variadic,
            is_void: matches!(kind, ValueKind::Void),
            return_native,
            return_class,
            enum_return: signature
                .return_enum
                .as_ref()
                .map(|name| format!("\\{}\\Enums\\{}", options.namespace, name)),
        });
    }

    super::render_inner_template(
        super::GLOBAL_FUNCTIONS_TEMPLATE,
        json!({
            "entity_fqcn": format!("\\{}\\{}", options.namespace, options.class_name),
            "functions": functions,
        }),
    )
}

fn is_php_reserved_function_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "__halt_compiler",
        "abstract",
        "and",
        "array",
        "as",
        "break",
        "callable",
        "case",
        "catch",
        "class",
        "clone",
        "const",
        "continue",
        "declare",
        "default",
        "die",
        "do",
        "echo",
        "else",
        "elseif",
        "empty",
        "enddeclare",
        "endfor",
        "endforeach",
        "endif",
        "endswitch",
        "endwhile",
        "enum",
        "eval",
        "exit",
        "extends",
        "final",
        "finally",
        "fn",
        "for",
        "foreach",
        "function",
        "global",
        "goto",
        "if",
        "implements",
        "include",
        "include_once",
        "instanceof",
        "insteadof",
        "interface",
        "isset",
        "list",
        "match",
        "namespace",
        "new",
        "or",
        "print",
        "private",
        "protected",
        "public",
        "readonly",
        "require",
        "require_once",
        "return",
        "static",
        "switch",
        "throw",
        "trait",
        "try",
        "unset",
        "use",
        "var",
        "while",
        "xor",
        "yield",
        "yield from",
    ];
    RESERVED.contains(&name.to_ascii_lowercase().as_str())
}

/// Distinct per-type pointer wrapper class names referenced by this package's
/// signatures (returns and parameters), so the generator can emit a class each.
pub(super) fn collect_pointer_types(options: &PhpPackageTemplateOptions<'_>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for signature in options.signatures {
        if let ValueKind::Pointer(Some(name)) = value_kind(&signature.return_type) {
            names.insert(name);
        }
        for param in &signature.params {
            if let ValueKind::Pointer(Some(name)) = value_kind(&param.type_name) {
                names.insert(name);
            }
        }
    }
    // Also emit a wrapper for every struct the cdef defines with a body, even when
    // no function takes/returns a pointer to it — a library may hand back such a
    // struct through `void *` (hiredis's `redisReply`, re-wrapped by the caller).
    // Match the signature-derived naming (`reserved_suffix`, the form
    // `pointer_type_name` produces) so a struct that IS named in a signature is not
    // emitted twice under two class names.
    for tag in options.struct_fields.keys() {
        names.insert(reserved_suffix(tag));
    }
    names.into_iter().collect()
}

fn method_view(
    prefix: &str,
    name: &str,
    signature: &FunctionSignature,
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
    native: bool,
) -> MethodView {
    // Arguments are marshalled by `\Pnlx\FFI\ArgumentMarshaller` (a static call, so
    // a C function named `unwrap`/`scalarArg` can't shadow it): pointers `unwrap`,
    // scalars `scalarArg` (which enforces `use_php_scalars_in_params`).
    let args: Vec<ArgView> = signature
        .params
        .iter()
        .map(|param| ArgView {
            name: param.name.clone(),
            is_pointer: matches!(
                value_kind(&param.type_name),
                ValueKind::Pointer(_) | ValueKind::Void
            ),
            is_native_pointer: pointer_out_param(&param.type_name).is_some()
                || writable_char_buffer(&param.type_name),
        })
        .collect();
    let has_out_params = args.iter().any(|arg| arg.is_native_pointer);

    let kind = value_kind(&signature.return_type);
    // The template builds `return new <new_class>(...)` for wrapped scalars and
    // pointer wrappers; the other cases are flagged directly.
    let new_class = match &kind {
        ValueKind::Int(wrapper) | ValueKind::Float(wrapper)
            if !(native && fits_php_scalar(wrapper)) =>
        {
            Some(format!("{HELPERS_NS}\\{}", wrapper.class))
        }
        ValueKind::Pointer(Some(class)) => Some(format!("{}\\{class}", types_ns(options))),
        ValueKind::Pointer(None) => Some(base_context(options)),
        _ => None,
    };
    let cast = match &kind {
        ValueKind::Int(wrapper) if native && fits_php_scalar(wrapper) => Some("int"),
        ValueKind::Float(wrapper) if native && fits_php_scalar(wrapper) => Some("float"),
        _ => None,
    };
    // An enum return takes its own `<Enum>::tryFrom(...)` form; the scalar/cast forms
    // (the collapsed `int`) must not also fire.
    let (new_class, cast) = if signature.return_enum.is_some() {
        (None, None)
    } else {
        (new_class, cast)
    };

    MethodView {
        name: format!("{prefix}{name}"),
        raw_name: signature.native_symbol().to_owned(),
        params: param_views(&signature.params, options, allow_cdata),
        call_name: name.to_owned(),
        args,
        variadic: signature.variadic,
        is_void: matches!(kind, ValueKind::Void),
        is_string: matches!(kind, ValueKind::Str),
        native_string: native,
        cast,
        new_class,
        enum_return: signature
            .return_enum
            .as_ref()
            .map(|name| format!("\\{}\\Enums\\{}", options.namespace, name)),
        has_out_params,
        unsupported_reason: signature.unsupported.clone(),
        unsupported_attribute: signature.unsupported.as_deref().map(unsupported_attribute),
    }
}

/// The fully-qualified marker attribute for an unsupported-function reason, built
/// in Rust because its leading backslash must not sit next to a `{{ }}` placeholder
/// (Handlebars treats `\{{` as an escape and would drop the backslash). Abstract
/// (no per-library knowledge): `static inline` -> `StaticInline`; any other reason
/// falls back to a generic marker so the API stays self-describing.
fn unsupported_attribute(reason: &str) -> String {
    let class = match reason {
        "static inline" => "StaticInline",
        _ => "Unsupported",
    };
    format!("\\Pnlx\\Attribute\\{class}")
}

/// Build the structured parameter views the template renders into accepted-type
/// unions. Parameters always accept a PHP scalar alongside the generated wrapper;
/// `allow_cdata` additionally admits a raw `\FFI\CData`.
fn param_views(
    params: &[FunctionParam],
    options: &PhpPackageTemplateOptions<'_>,
    allow_cdata: bool,
) -> Vec<ParamView> {
    let mut views: Vec<ParamView> = params
        .iter()
        .map(|param| {
            let mut view = ParamView {
                name: param.name.clone(),
                is_string: false,
                is_int: false,
                is_float: false,
                is_pointer: false,
                pointer_class: None,
                void_pointer: false,
                callback: false,
                enum_class: None,
                cdata: allow_cdata,
                native_pointer: false,
                np_element: String::new(),
                np_string: false,
                np_buffer: false,
                np_wrap: None,
                default_null: false,
            };
            // A C function-pointer parameter accepts a PHP `callable`; it still
            // marshals as a pointer (its `type_name` is `void *`), so it skips the
            // pointer/scalar type classification below.
            if param.callback {
                view.callback = true;
                return view;
            }
            // An enum parameter accepts the generated PHP enum alongside `int`; its
            // `type_name` is collapsed to `int`, so the normal scalar classification
            // below makes it `is_int`, and the marshaller sends the enum's backing
            // value. Only the accepted-type union changes.
            if let Some(enum_name) = &param.enum_type {
                view.enum_class = Some(format!("\\{}\\Enums\\{}", options.namespace, enum_name));
            }
            // A non-const `char *` is a writable byte buffer the call fills in: a
            // by-reference out-parameter the caller pre-sizes (it takes precedence
            // over the `const char *` string-input classification below).
            if writable_char_buffer(&param.type_name) {
                view.native_pointer = true;
                view.np_buffer = true;
                view.np_element = "char".to_owned();
                view.is_string = true;
                return view;
            }
            // A pointer the native call writes through (`int *`, `char **`, `T **`)
            // is a by-reference out/in-out parameter — it takes precedence over the
            // opaque-pointer classification value_kind would otherwise give it. The
            // `is_*` flag set here drives the accepted-type union the caller passes.
            if let Some(out) = pointer_out_param(&param.type_name) {
                view.native_pointer = true;
                match out {
                    PointerOut::Scalar(element) => {
                        view.np_element = element.to_owned();
                        match value_kind(element) {
                            ValueKind::Float(_) => view.is_float = true,
                            _ => view.is_int = true,
                        }
                    }
                    PointerOut::StringOut => {
                        // A `void *` holder is what the `char **`/`uint8_t **`
                        // out-parameter accepts at the FFI boundary (a `char **`
                        // holder is rejected as incompatible with `uint8_t **`). The
                        // written-back pointer is read as a C string by casting it to
                        // `char *` in the marshaller.
                        view.np_element = "void *".to_owned();
                        view.np_string = true;
                        view.is_string = true;
                    }
                    PointerOut::Handle => {
                        view.np_element = "void *".to_owned();
                        view.np_wrap = Some(base_context(options));
                        view.is_pointer = true;
                    }
                }
                return view;
            }
            match value_kind(&param.type_name) {
                ValueKind::Str => view.is_string = true,
                ValueKind::Int(_) => view.is_int = true,
                ValueKind::Float(_) => view.is_float = true,
                ValueKind::Pointer(Some(class)) => {
                    view.is_pointer = true;
                    view.pointer_class = Some(format!("{}\\{class}", types_ns(options)));
                }
                // A void parameter never occurs; treat it as an opaque pointer. A
                // single-level `void *` also accepts a PHP string (FFI passes its
                // bytes as the pointer), so flag it for the wider type union.
                ValueKind::Pointer(None) | ValueKind::Void => {
                    view.is_pointer = true;
                    view.void_pointer = is_void_pointer(&param.type_name);
                }
            }
            view
        })
        .collect();

    // Let a *trailing* run of out-parameters be omitted (`= null`), the
    // PHP-expressible equivalent of passing C `NULL` — `libusb_init()` instead of
    // the impossible `libusb_init(null)`. PHP forbids a default before a required
    // parameter, so only out-params with nothing-but-out-params after them qualify.
    for view in views.iter_mut().rev() {
        if view.native_pointer {
            view.default_null = true;
        } else {
            break;
        }
    }
    views
}
