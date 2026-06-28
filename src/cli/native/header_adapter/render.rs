//! Assembling the final cdef text from the collected declarations: emitting
//! functions/globals/typedefs/structs/unions in dependency order, backfilling
//! missing pointer/function types, filtering to exported symbols, and the type/
//! identifier-name analysis that supports all of it. Entry point: `render`.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

pub(super) fn render(collected: &Collected, exported_symbols: Option<&BTreeSet<String>>) -> String {
    // An enum *usage* keeps its bare typedef name ONLY in function declarations, so
    // `parse_function_signatures` can tag enum-typed params/returns for the PHP-enum
    // layer (the name still resolves to `int` via the emitted `typedef int <enum>`).
    // Everywhere else (typedefs, globals, struct/union bodies) it must project to
    // `int`: keeping the name in the enum's OWN forward typedef would emit
    // `typedef <enum> <enum>` (self-referential, conflicts with `typedef int <enum>`)
    // and break `FFI::cdef` — notcurses's `ncintype_e`/`ncblitter_e`.
    let no_enums: BTreeSet<String> = BTreeSet::new();
    let mut typedefs = collected
        .typedefs
        .iter()
        .map(|typedef| project_enums_to_int(typedef, &no_enums))
        .collect::<Vec<_>>();
    // For resolving byte-pointer parameters hidden behind a typedef.
    let byte_typedefs = simple_typedef_map(collected);
    // Every declared type name, so a parameter that shadows one can be renamed
    // (the cdef parser would otherwise read the name as a second type).
    let declared_type_names: BTreeSet<String> = collected
        .typedef_names
        .iter()
        .chain(collected.enums.iter())
        .chain(collected.structs.keys())
        .chain(collected.struct_aliases.keys())
        .chain(collected.unions.keys())
        .chain(collected.union_aliases.keys())
        .chain(collected.pool_typedefs.keys())
        .chain(collected.pool_struct_aliases.keys())
        .chain(collected.pool_union_aliases.keys())
        .chain(collected.pool_enums.iter())
        .cloned()
        .collect();
    // Every externally-linked function declared in the package's own header(s)
    // is part of its API. The declarations are already scoped to the main file
    // (system/#included headers are excluded during collection), so there is no
    // need to additionally filter by symbol prefix — and doing so wrongly drops
    // functions for libraries without a uniform prefix (e.g. zlib's `deflate`,
    // `compress`, `crc32`). When the installed library's exported symbols are
    // known, drop declarations it does not provide so the FFI load can't fail on
    // a single absent symbol (version/build skew).
    let functions = collected
        .functions
        .iter()
        .filter(|function| match exported_symbols {
            Some(symbols) => function_decl_name(function).is_none_or(|name| symbols.contains(name)),
            None => true,
        })
        .map(|function| project_enums_to_int(function, &collected.enums))
        .map(|function| rewrite_byte_pointer_params(&function, &byte_typedefs))
        .map(|function| rename_type_colliding_params(&function, &declared_type_names))
        .collect::<Vec<_>>();

    // Exported global variables, dropped when the installed `.so` doesn't export the
    // symbol (same version/build-skew guard as functions).
    let globals = collected
        .globals
        .iter()
        .filter(|global| match exported_symbols {
            Some(symbols) => global_decl_name(global).is_none_or(|name| symbols.contains(&name)),
            None => true,
        })
        .map(|global| project_enums_to_int(global, &no_enums))
        .collect::<Vec<_>>();

    // Resolve types referenced by the kept declarations but defined only in
    // #included headers (e.g. zlib's `voidpf`/`uLong` from `zconf.h`), pulling
    // their real definitions from the pool so the cdef is self-contained. These
    // go before the package's own typedefs, which may reference them.
    let pool_resolved = resolve_pool_types(&typedefs, &functions, collected);
    let resolved_names = pool_resolved_names(&pool_resolved);
    let mut typedefs_with_pool = pool_resolved;
    typedefs_with_pool.append(&mut typedefs);
    // Emit each typedef name at most once: a type can be both pooled (from an
    // #included header) and declared in the package's own header, which would
    // otherwise produce a "Redeclaration of …" cdef error. On a name clash,
    // prefer a "real" (pointer/record/scalar) typedef over a function-type one,
    // which libclang's error recovery can emit as a spurious duplicate.
    let mut chosen: BTreeMap<String, usize> = BTreeMap::new();
    let mut typedefs: Vec<String> = Vec::new();
    for decl in typedefs_with_pool {
        match typedef_defined_name(&decl) {
            Some(name) => match chosen.get(&name).copied() {
                None => {
                    chosen.insert(name, typedefs.len());
                    typedefs.push(decl);
                }
                Some(index) => {
                    if is_function_typedef(&typedefs[index]) && !is_function_typedef(&decl) {
                        typedefs[index] = decl;
                    }
                }
            },
            None => typedefs.push(decl),
        }
    }
    // A typedef may reference another typedef's name in its body (e.g. SQLite's
    // `typedef sqlite_uint64 sqlite3_uint64;`), so order them dependencies-first
    // rather than alphabetically — C requires a type to be defined before use.
    let typedefs = order_typedefs(typedefs);
    let typedefs: Vec<String> = typedefs
        .iter()
        .map(|decl| degrade_unsizable_array_typedef(decl, collected))
        .map(|decl| rewrite_unsupported_scalar_typedef(&decl))
        .collect();

    let missing_types = missing_function_types(&functions, &typedefs, collected, &resolved_names);
    let referenced = referenced_structs(&typedefs, &functions, &collected.structs);

    let mut out = String::new();
    out.push_str(PRELUDE);
    out.push('\n');

    out.push_str("struct timeval;\n");
    let alias_tags = collected
        .struct_aliases
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    for name in referenced
        .iter()
        .chain(collected.structs.keys())
        .chain(alias_tags.iter())
    {
        out.push_str("struct ");
        out.push_str(name);
        out.push_str(";\n");
    }
    let union_alias_tags = collected
        .union_aliases
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    for name in collected.unions.keys().chain(union_alias_tags.iter()) {
        out.push_str("union ");
        out.push_str(name);
        out.push_str(";\n");
    }
    for name in missing_types
        .iter()
        .filter_map(|(name, kind)| matches!(kind, MissingTypeKind::OpaqueStruct).then_some(name))
    {
        out.push_str("struct ");
        out.push_str(name);
        out.push_str(";\n");
    }

    // A struct/union/enum tag may share its spelling with a function or a real
    // typedef (C keeps tags in a separate namespace; PHP FFI does not). Emitting
    // the convenience `typedef <tag> <tag>;` then collides ("Redeclaration"), so
    // skip it — such types are still reachable through `struct <tag>`/`enum <tag>`
    // (soxr's `soxr_quality_spec`, gnutls's `gnutls_random_art`, popt's
    // `poptOption`, which already has a real pointer typedef).
    let self_typedef_collides = |name: &str| {
        collected.function_names.contains(name) || collected.typedef_names.contains(name)
    };

    out.push('\n');
    for name in collected.structs.keys() {
        if self_typedef_collides(name) {
            continue;
        }
        out.push_str(&format!("typedef struct {name} {name};\n"));
    }
    for (alias, tag) in &collected.struct_aliases {
        if collected.structs.contains_key(alias) && alias == tag {
            continue;
        }
        out.push_str(&format!("typedef struct {tag} {alias};\n"));
    }
    for (alias, tag) in &collected.union_aliases {
        if collected.unions.contains_key(alias) && alias == tag {
            continue;
        }
        out.push_str(&format!("typedef union {tag} {alias};\n"));
    }
    for name in collected.unions.keys() {
        if self_typedef_collides(name) {
            continue;
        }
        out.push_str(&format!("typedef union {name} {name};\n"));
    }
    for name in missing_types
        .iter()
        .filter_map(|(name, kind)| matches!(kind, MissingTypeKind::OpaqueStruct).then_some(name))
    {
        out.push_str(&format!("typedef struct {name} {name};\n"));
    }
    for name in &collected.enums {
        if self_typedef_collides(name) {
            continue;
        }
        out.push_str(&format!("typedef int {name};\n"));
    }
    for name in missing_types
        .iter()
        .filter_map(|(name, kind)| matches!(kind, MissingTypeKind::Int).then_some(name))
    {
        out.push_str(&format!("typedef int {name};\n"));
    }
    for typedef in &typedefs {
        out.push_str(typedef);
        out.push('\n');
    }

    // The structs/unions the cdef emits with a full body (everything else stays
    // forward-declared/opaque). Computed once: a union must be structurally safe and
    // every by-value aggregate member must itself be emittable (transitively).
    let (emit_structs, emit_unions) = emittable_aggregates(collected);

    // Every type name the cdef will emit, including pool types resolved on demand
    // (`resolved_names`) and the final typedef set. Used to detect an aggregate whose
    // field name was corrupted into a type token (see `body_has_type_named_field`):
    // emitting it would make PHP FFI reject the whole cdef, so keep it opaque.
    let mut all_type_names = declared_type_names.clone();
    all_type_names.extend(resolved_names.iter().cloned());
    all_type_names.extend(typedefs.iter().filter_map(|d| typedef_defined_name(d)));
    all_type_names.extend(missing_types.keys().cloned());
    // PHP FFI's intrinsic type names (`off_t`, `time_t`, `size_t`, …) are never
    // emitted as typedefs but still collide with a same-named param/field.
    all_type_names.extend(builtin_type_names());

    // Re-run the parameter/type collision rename against the COMPLETE type set: the
    // earlier pass used `declared_type_names`, which lacks pool-resolved and
    // `missing_types` names. An unnamed param of such a type takes the type's
    // spelling as its name (libarchive's `archive_entry_set_atime(…, __LA_TIME_T, …)`
    // → `int time_t`), and `time_t` is also an emitted typedef, so PHP FFI would
    // reject `int time_t;`. Renaming the colliding param keeps the cdef valid.
    let functions: Vec<String> = functions
        .iter()
        .map(|function| rename_type_colliding_params(function, &all_type_names))
        .collect();

    out.push('\n');
    out.push_str("struct timeval { long tv_sec; int tv_usec; };\n");
    for (name, definition) in &collected.unions {
        if !emit_unions.contains(name) {
            continue;
        }
        if body_has_type_named_field(definition, &all_type_names) {
            continue;
        }
        out.push_str(&project_enums_to_int(definition, &no_enums));
        out.push('\n');
    }
    for definition in ordered_struct_definitions(collected) {
        // A struct with a by-value member of a type the cdef leaves incomplete
        // (an embedded system `struct sockaddr_in`, or an unsafe union) can't be
        // laid out; keep it opaque (forward-declared only).
        if struct_definition_tag(definition).is_none_or(|tag| !emit_structs.contains(tag)) {
            continue;
        }
        if body_has_type_named_field(definition, &all_type_names) {
            continue;
        }
        out.push_str(&project_enums_to_int(definition, &no_enums));
        out.push('\n');
    }

    out.push('\n');
    for function in &functions {
        out.push_str(function);
        out.push('\n');
    }

    for global in &globals {
        out.push_str(global);
        out.push('\n');
    }

    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingTypeKind {
    Int,
    OpaqueStruct,
}

/// Resolve types referenced by the kept typedefs/functions but defined only in
/// #included headers, pulling their definitions out of the pool. Returns typedef
/// declarations in dependency order (a type's own dependencies first), so the
/// list can be emitted ahead of the package's own typedefs.
fn resolve_pool_types(
    typedefs: &[String],
    functions: &[String],
    collected: &Collected,
) -> Vec<String> {
    let known = known_type_names(collected);

    fn visit(
        name: &str,
        collected: &Collected,
        known: &BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        out: &mut Vec<String>,
    ) {
        if known.contains(name) || !visited.insert(name.to_owned()) {
            return;
        }
        if let Some(underlying) = collected.pool_typedefs.get(name) {
            for token in identifier_tokens(underlying) {
                visit(token, collected, known, visited, out);
            }
            let underlying = strip_unsizable_array(underlying, collected);
            out.push(typedef_declaration(name, &underlying));
        } else if let Some(tag) = collected.pool_struct_aliases.get(name) {
            out.push(format!("typedef struct {tag} {name};"));
        } else if let Some(tag) = collected.pool_union_aliases.get(name) {
            out.push(format!("typedef union {tag} {name};"));
        } else if collected.pool_enums.contains(name) {
            out.push(format!("typedef int {name};"));
        }
        // Not in the pool: leave it for the int/opaque-struct fallback.
    }

    let mut visited = BTreeSet::new();
    let mut out = Vec::new();
    for declaration in typedefs
        .iter()
        .chain(functions.iter())
        .chain(collected.structs.values())
        .chain(collected.unions.values())
    {
        for token in identifier_tokens(declaration) {
            visit(token, collected, &known, &mut visited, &mut out);
        }
    }
    out
}

/// Order typedef declarations so each one appears after any other typedef it
/// references in its body (topological sort). Preserves the original order among
/// independent typedefs. Cycles (rare/illegal in C) fall back to insertion order.
fn order_typedefs(typedefs: Vec<String>) -> Vec<String> {
    // name -> (index, decl); index keeps a stable order for independents.
    let mut by_name: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut unnamed = Vec::new();
    for (index, decl) in typedefs.iter().enumerate() {
        match typedef_defined_name(decl) {
            Some(name) => {
                by_name.entry(name).or_insert((index, decl.clone()));
            }
            None => unnamed.push(decl.clone()),
        }
    }

    fn visit(
        name: &str,
        by_name: &BTreeMap<String, (usize, String)>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if visited.contains(name) || !visiting.insert(name.to_owned()) {
            return;
        }
        if let Some((_, decl)) = by_name.get(name) {
            for token in identifier_tokens(decl) {
                if token != name && by_name.contains_key(token) {
                    visit(token, by_name, visiting, visited, ordered);
                }
            }
            ordered.push(decl.clone());
        }
        visiting.remove(name);
        visited.insert(name.to_owned());
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    // Iterate in the original index order for determinism.
    let mut names: Vec<&String> = by_name.keys().collect();
    names.sort_by_key(|name| by_name[*name].0);
    for name in names {
        visit(name, &by_name, &mut visiting, &mut visited, &mut ordered);
    }
    ordered.extend(unnamed);
    ordered
}

/// Drop the array dimension from a pooled typedef whose element is a struct/union
/// tag that stays opaque in the cdef (no body is emitted for it). PHP FFI needs
/// the element size to lay out an array, so `typedef struct __jmp_buf_tag
/// jmp_buf[1];` over an incomplete `__jmp_buf_tag` fails to parse. Such types
/// (e.g. `jmp_buf`) are only ever used through a pointer, for which the element
/// size is irrelevant, so the alias to the incomplete struct is enough.
fn strip_unsizable_array(underlying: &str, collected: &Collected) -> String {
    let Some(open) = underlying.find('[') else {
        return underlying.to_owned();
    };
    let base = underlying[..open].trim_end();
    if is_incomplete_aggregate(base, collected) {
        base.to_owned()
    } else {
        underlying.to_owned()
    }
}

/// Whether a type name denotes a struct/union the cdef leaves incomplete (only
/// forward-declared, no body) — directly (`struct X`/`union X`) or through a
/// typedef alias (`__gmp_randstate_struct`, libreadline's `KEYMAP_ENTRY`). PHP
/// FFI can't size such a type for a by-value array element.
fn is_incomplete_aggregate(name: &str, collected: &Collected) -> bool {
    if let Some(tag) = name.strip_prefix("struct ") {
        return !collected.structs.contains_key(tag);
    }
    if let Some(tag) = name.strip_prefix("union ") {
        return !collected.unions.contains_key(tag);
    }
    if let Some(tag) = collected
        .struct_aliases
        .get(name)
        .or_else(|| collected.pool_struct_aliases.get(name))
    {
        return !collected.structs.contains_key(tag);
    }
    if let Some(tag) = collected
        .union_aliases
        .get(name)
        .or_else(|| collected.pool_union_aliases.get(name))
    {
        return !collected.unions.contains_key(tag);
    }
    false
}

/// Render-time pass: degrade `typedef <elem> name[N];` to `typedef <elem> name;`
/// when `<elem>` is an aggregate (struct/union) — libgmp `mpz_t[1]`, libtheora
/// `th_ycbcr_buffer[3]`, libreadline `KEYMAP_ENTRY_ARRAY[257]`. These are the C
/// by-reference array idiom; the typedef is always used through a pointer (an
/// array parameter decays anyway), so dropping the dimension is functionally
/// transparent and sidesteps both "incomplete struct" and definition-ordering
/// parse errors. A scalar-element buffer (`unsigned char th_quant_base[64]`)
/// keeps its array, since those are real fixed-size byte buffers.
fn degrade_unsizable_array_typedef(decl: &str, collected: &Collected) -> String {
    if !decl.contains('[') {
        return decl.to_owned();
    }
    let Some(inner) = decl.strip_prefix("typedef ") else {
        return decl.to_owned();
    };
    let inner = inner.trim_end_matches(';').trim();
    let Some(open) = inner.find('[') else {
        return decl.to_owned();
    };
    let Some((element, name)) = split_declaration_name(inner[..open].trim()) else {
        return decl.to_owned();
    };
    if is_aggregate_type(&element, collected) {
        format!("typedef {element} {name};")
    } else {
        decl.to_owned()
    }
}

/// Render-time string form of [`unsupported_scalar_typedef`], for typedefs that
/// reach the cdef through the pool resolver (where only the textual underlying is
/// available, e.g. openblas's `typedef _Complex double openblas_complex_double;`).
fn rewrite_unsupported_scalar_typedef(decl: &str) -> String {
    let parsed = decl
        .strip_prefix("typedef ")
        .and_then(|inner| Some((inner, typedef_defined_name(decl)?)));
    let Some((inner, name)) = parsed else {
        return decl.to_owned();
    };
    if inner.contains("__int128") {
        return format!("typedef struct {{ uint64_t lo; uint64_t hi; }} {name};");
    }
    if inner.contains("_Complex") {
        let element = if inner.contains("float") {
            "float"
        } else {
            "double"
        };
        return format!("typedef struct {{ {element} re; {element} im; }} {name};");
    }
    decl.to_owned()
}

/// Whether a type name denotes a struct/union (complete or not), directly or via
/// a typedef alias.
fn is_aggregate_type(name: &str, collected: &Collected) -> bool {
    name.starts_with("struct ")
        || name.starts_with("union ")
        || collected.struct_aliases.contains_key(name)
        || collected.pool_struct_aliases.contains_key(name)
        || collected.union_aliases.contains_key(name)
        || collected.pool_union_aliases.contains_key(name)
}

/// The names defined by `resolve_pool_types`' output (the identifier of each
/// `typedef … <name>;`).
fn pool_resolved_names(resolved: &[String]) -> BTreeSet<String> {
    resolved
        .iter()
        .filter_map(|d| typedef_defined_name(d))
        .collect()
}

/// The identifier a `typedef …;` declaration introduces, handling plain
/// (`typedef unsigned long uLong;`), function-pointer (`typedef void (*cb)(int);`)
/// and function-type (`typedef int handler(void *);`) forms.
fn typedef_defined_name(decl: &str) -> Option<String> {
    let inner = decl.strip_prefix("typedef ")?.trim_end_matches(';').trim();
    if let Some(open) = inner.find("(*") {
        // Function-pointer typedef: the name follows `(*` up to `)`.
        let rest = &inner[open + 2..];
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    if let Some(open) = inner.find('(') {
        // Function-type typedef (`int handler(args)`): the name is the identifier
        // immediately before the parameter list.
        let before = inner[..open].trim_end();
        let start = before
            .rfind(|ch: char| !is_c_identifier_char(ch))
            .map_or(0, |index| index + 1);
        let name = &before[start..];
        return (!name.is_empty()).then_some(name.to_owned());
    }
    // Array typedef (`struct __jmp_buf_tag jmp_buf[1];`): the name precedes the
    // `[`, so drop the array declarator before extracting it.
    let inner = match inner.find('[') {
        Some(open) => inner[..open].trim_end(),
        None => inner,
    };
    split_declaration_name(inner).map(|(_, name)| name)
}

/// Whether a typedef declaration is a function-type form (`typedef ret name(args);`).
/// libclang's error recovery can emit such a typedef as a spurious duplicate of a
/// real (pointer/record) typedef of the same name; the real one should win.
fn is_function_typedef(decl: &str) -> bool {
    let Some(inner) = decl.strip_prefix("typedef ") else {
        return false;
    };
    !inner.contains("(*") && inner.contains('(')
}

fn missing_function_types(
    functions: &[String],
    typedefs: &[String],
    collected: &Collected,
    resolved_names: &BTreeSet<String>,
) -> BTreeMap<String, MissingTypeKind> {
    let known = known_type_names(collected);
    let mut missing = BTreeMap::new();
    // `allow_plain_lowercase` is set only when scanning a function-pointer parameter's
    // component types, which are pure type positions (the generator renders them with
    // no parameter names) — so an all-lowercase token there is a real type, not a
    // dropped parameter name, and is safe to backfill.
    let mut scan = |fragment: &str, allow_plain_lowercase: bool| {
        for (start, token) in identifier_spans(fragment) {
            if is_c_keyword(token)
                || known.contains(token)
                || resolved_names.contains(token)
                || collected.function_names.contains(token)
                || (!allow_plain_lowercase && !looks_like_external_type_name(token))
            {
                continue;
            }
            // A token right after a `struct`/`union`/`enum` keyword is a tag,
            // already forward-declared by `referenced_structs`; not a typedef.
            if matches!(
                last_identifier(&fragment[..start]),
                "struct" | "union" | "enum"
            ) {
                continue;
            }
            let before = previous_nonspace(fragment, start);
            if before == Some('*') {
                continue;
            }
            let after = next_nonspace(fragment, start + token.len());
            let kind = if after == Some('*') {
                MissingTypeKind::OpaqueStruct
            } else {
                MissingTypeKind::Int
            };
            missing
                .entry(token.to_owned())
                .and_modify(|existing| {
                    if kind == MissingTypeKind::Int {
                        *existing = MissingTypeKind::Int;
                    }
                })
                .or_insert(kind);
        }
    };
    for function in functions {
        for type_part in function_type_parts(function) {
            if type_part.contains('(') || type_part.contains(')') {
                continue;
            }
            scan(&type_part, false);
        }
        // A type named only inside a function-pointer *parameter* (the callback's own
        // return/argument types) sits in parentheses, so `function_type_parts` skips
        // it. Scan those components directly so a struct/typedef reached only through a
        // callback signature is still backfilled and the cdef keeps loading.
        for component in callback_component_types(function) {
            scan(&component, true);
        }
    }
    // Also scan typedefs, so a type referenced only inside a function-pointer
    // typedef (mbedtls's `mbedtls_x509_buf` in a callback signature) is backfilled.
    // The typedef's own name is already in `known` (collected.typedef_names).
    for typedef in typedefs {
        scan(typedef, false);
    }
    missing
}

/// The component types inside every function-pointer parameter of a cdef function
/// declaration — the callback's own return type and argument types. These live
/// inside the parameter's parentheses, so the top-level [`function_type_parts`] scan
/// never sees them. The generator renders these components with no parameter names,
/// so each returned string is a bare type (`const point *`, `int`).
pub(super) fn callback_component_types(function: &str) -> Vec<String> {
    let Some(open) = function.find('(') else {
        return Vec::new();
    };
    let Some(close) = function.rfind(')') else {
        return Vec::new();
    };
    let mut components = Vec::new();
    for param in split_comma_separated(&function[open + 1..close]) {
        // A function-pointer parameter is `RET (*name)(ARGS)`; the `(*` opens its
        // declarator. Anything without it is an ordinary parameter handled elsewhere.
        let Some(star) = param.find("(*") else {
            continue;
        };
        let return_type = param[..star].trim();
        if !return_type.is_empty() && return_type != "void" {
            components.push(return_type.to_owned());
        }
        // Skip the `(*name)` declarator group, then read the argument list that
        // follows it.
        let after_declarator = &param[star..];
        let Some(declarator_close) = after_declarator.find(')') else {
            continue;
        };
        let rest = &after_declarator[declarator_close + 1..];
        let (Some(args_open), Some(args_close)) = (rest.find('('), rest.rfind(')')) else {
            continue;
        };
        for argument in split_comma_separated(&rest[args_open + 1..args_close]) {
            let argument = argument.trim();
            if !argument.is_empty() && argument != "void" && argument != "..." {
                components.push(argument.to_owned());
            }
        }
    }
    components
}

/// Drop single-line function declarations the resolved library does not export
/// from a verbatim (empty-`symbol_prefix`) header. Only lines that are clearly a
/// single-line function declaration are considered (end in `;`, contain `(`, carry
/// no aggregate body, and are not a `typedef`/tag definition); everything else —
/// typedefs, struct/union/enum, blank lines, and any function whose return type is
/// a `struct`/`union`/`enum` (conservatively kept) — passes through unchanged. This
/// is the empty-prefix counterpart to the export filter in [`render`].
pub(super) fn filter_verbatim_to_exports(header: &str, exported: &BTreeSet<String>) -> String {
    let mut out = String::new();
    for line in header.lines() {
        let trimmed = line.trim();
        let looks_like_function = trimmed.ends_with(';')
            && trimmed.contains('(')
            && !trimmed.contains('{')
            && !trimmed.starts_with("typedef ")
            && !trimmed.starts_with("struct ")
            && !trimmed.starts_with("union ")
            && !trimmed.starts_with("enum ");
        let drop = looks_like_function
            && function_decl_name(trimmed).is_some_and(|name| !exported.contains(name));
        if !drop {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The function name in a cdef declaration like `int foo(int a);` — the
/// identifier immediately before the parameter list.
fn function_decl_name(function: &str) -> Option<&str> {
    let open = function.find('(')?;
    let before = function[..open].trim_end();
    let start = before
        .rfind(|ch: char| !is_c_identifier_char(ch))
        .map_or(0, |index| index + 1);
    let name = &before[start..];
    (!name.is_empty()).then_some(name)
}

/// The variable name in a global declaration like `extern T name;` — the trailing
/// identifier, so the export filter can check the symbol.
pub(super) fn global_decl_name(global: &str) -> Option<String> {
    let inner = global.strip_prefix("extern ")?.trim_end_matches(';').trim();
    split_declaration_name(inner).map(|(_, name)| name)
}

fn function_type_parts(function: &str) -> Vec<String> {
    let Some(open) = function.find('(') else {
        return Vec::new();
    };
    let Some(close) = function.rfind(')') else {
        return Vec::new();
    };

    let mut parts = Vec::new();
    if let Some((return_type, _)) = split_declaration_name(function[..open].trim()) {
        parts.push(return_type);
    }

    for param in split_comma_separated(&function[open + 1..close]) {
        if param == "void" || param == "..." {
            continue;
        }
        if let Some((type_name, _)) = split_declaration_name(param) {
            parts.push(type_name);
        }
    }
    parts
}

pub(super) fn split_declaration_name(declaration: &str) -> Option<(String, String)> {
    let declaration = declaration.trim().trim_end_matches(';').trim();
    let end = declaration
        .char_indices()
        .rev()
        .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index + ch.len_utf8()))?;
    let trimmed = &declaration[..end];
    let mut start = trimmed.len();
    for (index, ch) in trimmed.char_indices().rev() {
        if is_c_identifier_char(ch) {
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
    (!type_name.is_empty()).then_some((type_name, name))
}

fn split_comma_separated(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                parts.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn known_type_names(collected: &Collected) -> BTreeSet<String> {
    let mut known = builtin_type_names();
    known.extend(collected.structs.keys().cloned());
    known.extend(collected.struct_aliases.keys().cloned());
    known.extend(collected.unions.keys().cloned());
    known.extend(collected.union_aliases.keys().cloned());
    known.extend(collected.enums.iter().cloned());
    known.extend(collected.typedef_names.iter().cloned());
    known
}

pub(super) fn builtin_type_names() -> BTreeSet<String> {
    [
        "const",
        "volatile",
        "restrict",
        "signed",
        "unsigned",
        "char",
        "short",
        "int",
        "long",
        "float",
        "double",
        "void",
        "bool",
        "_Bool",
        "size_t",
        "ssize_t",
        "intptr_t",
        "uintptr_t",
        // PHP FFI knows these intrinsically (no typedef needed/allowed); a param or
        // field named after one — zlib's unnamed `z_off_t` param becomes `int off_t`
        // — must be renamed or the cdef is rejected ("unexpected '<ID>'"). NOTE:
        // `time_t` is NOT here — PHP FFI does NOT know it, so it must keep its emitted
        // `typedef int time_t;` (suppressing it breaks libarchive's `time_t` returns).
        "off_t",
        "ptrdiff_t",
        "uint8_t",
        "uint16_t",
        "uint32_t",
        "uint64_t",
        "int8_t",
        "int16_t",
        "int32_t",
        "int64_t",
        "Uint8",
        "Uint16",
        "Uint32",
        "Uint64",
        "Sint8",
        "Sint16",
        "Sint32",
        "Sint64",
        "wchar_t",
        "va_list",
        // Non-standard but widespread integer aliases (SysV/BSD), defined in the
        // prelude. Real libraries use them in their public API (HTML Tidy's `uint`)
        // but their `typedef` often lives in a platform header libclang never sees.
        "uchar",
        "ushort",
        "uint",
        "ulong",
        "u_char",
        "u_short",
        "u_int",
        "u_long",
        // GLib fundamental types, defined in the prelude (their real definitions
        // live in a libdir `glibconfig.h` libclang never sees). Listed here so the
        // missing-type backfill treats them as known and never re-emits them.
        "gchar",
        "guchar",
        "gshort",
        "gushort",
        "gint",
        "guint",
        "glong",
        "gulong",
        "gboolean",
        "gint8",
        "guint8",
        "gint16",
        "guint16",
        "gint32",
        "guint32",
        "gint64",
        "guint64",
        "gsize",
        "gssize",
        "goffset",
        "gintptr",
        "guintptr",
        "gunichar",
        "gunichar2",
        "gfloat",
        "gdouble",
        "gpointer",
        "gconstpointer",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Whether an unresolved identifier in a type position is plausibly an undefined
/// *type* worth backfilling. Kept conservative — `FILE`, names with an underscore,
/// or any name carrying an uppercase letter (`TIFFFieldInfo`, `mbedtls_x509_buf`,
/// camelCase `xmlXPathObjectPtr`). A bare *all-lowercase* token is rejected: it is
/// usually a dropped parameter *name* (libclang leaves `order` when it can't
/// resolve `CBLAS_ORDER order`), which must not become a spurious `typedef int
/// order;` that collides with a generated class.
fn looks_like_external_type_name(token: &str) -> bool {
    token == "FILE" || token.contains('_') || token.chars().any(|ch| ch.is_ascii_uppercase())
}

/// C keywords that can appear in a type position but are never themselves a type
/// name to backfill. (Type keywords like `int`/`const` are in `builtin_type_names`.)
fn is_c_keyword(token: &str) -> bool {
    matches!(
        token,
        "struct"
            | "union"
            | "enum"
            | "typedef"
            | "return"
            | "static"
            | "extern"
            | "inline"
            | "sizeof"
            | "_Complex"
    )
}

/// The last C identifier in a string (e.g. the keyword preceding a tag), or "".
fn last_identifier(input: &str) -> &str {
    let trimmed = input.trim_end();
    let start = trimmed
        .rfind(|ch: char| !is_c_identifier_char(ch))
        .map_or(0, |index| index + 1);
    &trimmed[start..]
}

fn identifier_spans(input: &str) -> Vec<(usize, &str)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, ch) in input.char_indices() {
        if is_c_identifier_char(ch) {
            start.get_or_insert(index);
        } else if let Some(token_start) = start.take() {
            spans.push((token_start, &input[token_start..index]));
        }
    }
    if let Some(token_start) = start {
        spans.push((token_start, &input[token_start..]));
    }
    spans
}

fn previous_nonspace(input: &str, index: usize) -> Option<char> {
    input[..index].chars().rev().find(|ch| !ch.is_whitespace())
}

fn next_nonspace(input: &str, index: usize) -> Option<char> {
    input[index..].chars().find(|ch| !ch.is_whitespace())
}

/// The set of struct tags and union tags the cdef will actually EMIT with a full
/// body, computed once for the whole package by fixpoint. A union must be
/// structurally safe (reference no struct and no other union) AND every aggregate
/// any kept aggregate embeds BY VALUE must itself be emitted — so a struct embedding
/// a union (`JSValue` ← `JSValueUnion`) is emitted only if that union is, and one
/// embedding an unsafe union (`config_setting_t` ← `config_value_t`) is not. Doing
/// this as a fixpoint (vs per-aggregate recursion) keeps it cheap on big headers.
fn emittable_aggregates(collected: &Collected) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut structs: BTreeSet<String> = collected.structs.keys().cloned().collect();
    let mut unions: BTreeSet<String> = collected
        .unions
        .iter()
        .filter(|(name, definition)| union_structurally_safe(name, definition, collected))
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let drop_unions: Vec<String> = unions
            .iter()
            .filter(|name| {
                aggregate_has_unemittable_member(
                    &collected.unions[*name],
                    collected,
                    &structs,
                    &unions,
                )
            })
            .cloned()
            .collect();
        let drop_structs: Vec<String> = structs
            .iter()
            .filter(|name| {
                aggregate_has_unemittable_member(
                    &collected.structs[*name],
                    collected,
                    &structs,
                    &unions,
                )
            })
            .cloned()
            .collect();
        if drop_unions.is_empty() && drop_structs.is_empty() {
            break;
        }
        for name in drop_unions {
            unions.remove(&name);
        }
        for name in drop_structs {
            structs.remove(&name);
        }
    }
    (structs, unions)
}

/// The tag of a `struct TAG { … };` definition (the identifier after `struct `),
/// or `None` if the string is not a named struct definition.
fn struct_definition_tag(definition: &str) -> Option<&str> {
    let rest = definition.trim_start().strip_prefix("struct ")?;
    let tag = rest.trim_start();
    let end = tag
        .find(|ch: char| !is_c_identifier_char(ch))
        .unwrap_or(tag.len());
    (end > 0).then(|| &tag[..end])
}

/// Whether a union's body references no `struct`/`struct`-alias (PHP FFI can't place
/// those in a union here) and no OTHER union — a fixed structural prerequisite for
/// emitting the union, independent of which aggregates end up emittable.
fn union_structurally_safe(name: &str, definition: &str, collected: &Collected) -> bool {
    for candidate in collected
        .structs
        .keys()
        .chain(collected.struct_aliases.keys())
    {
        if contains_c_identifier(definition, candidate) {
            return false;
        }
    }
    for candidate in collected
        .unions
        .keys()
        .chain(collected.union_aliases.keys())
    {
        if candidate != name && contains_c_identifier(definition, candidate) {
            return false;
        }
    }
    true
}

/// Whether an aggregate body has a *by-value* member of a `struct`/`union` NOT in
/// the emittable sets — a forward-declared system type (`struct sockaddr_in`), an
/// unsafe union (`config_value_t`), or a struct itself incomplete. PHP FFI can't
/// size such a member, so the enclosing aggregate must stay opaque. Pointer members
/// are fine and ignored. Non-recursive: the sets already capture transitive
/// emittability (see [`emittable_aggregates`]).
fn aggregate_has_unemittable_member(
    definition: &str,
    collected: &Collected,
    structs: &BTreeSet<String>,
    unions: &BTreeSet<String>,
) -> bool {
    // Scan the MEMBERS only — skip the aggregate's own `struct/union NAME {` header,
    // or its leading tag would be read as a by-value member of itself. Members all
    // sit after the first `{`.
    let definition = definition
        .find('{')
        .map_or(definition, |brace| &definition[brace..]);
    for keyword in ["struct", "union"] {
        let mut search_from = 0;
        while let Some(found) = definition[search_from..].find(keyword) {
            let index = search_from + found;
            search_from = index + keyword.len();
            // Require word boundaries so `struct` doesn't match inside an identifier.
            let before_ok = definition[..index]
                .chars()
                .next_back()
                .is_none_or(|ch| !is_c_identifier_char(ch));
            let after = &definition[search_from..];
            let after_ok = after
                .chars()
                .next()
                .is_some_and(|ch| !is_c_identifier_char(ch));
            if !before_ok || !after_ok {
                continue;
            }
            let after = after.trim_start();
            let tag: String = after
                .chars()
                .take_while(|ch| is_c_identifier_char(*ch))
                .collect();
            if tag.is_empty() {
                continue; // anonymous inline `struct { … }`
            }
            // A pointer member needs no size; only by-value members must be sized.
            if after[tag.len()..].trim_start().starts_with('*') {
                continue;
            }
            // "Complete" means actually EMITTED — present in the precomputed set.
            let complete = if keyword == "struct" {
                structs.contains(&tag)
            } else {
                unions.contains(&tag)
            };
            if !complete {
                return true;
            }
        }
    }
    // A by-value member typed as a bare *typedef* (no `struct`/`union` keyword) that
    // aliases an aggregate the cdef won't emit — mbedtls embeds the incomplete
    // `mbedtls_x509_san_other_name` by value; libconfig embeds the unsafe union alias
    // `config_value_t` by value. Pointer members (`alias *p`) need no size, ignored.
    let incomplete_struct_aliases = collected
        .struct_aliases
        .iter()
        .filter(|(_, tag)| !structs.contains(*tag))
        .map(|(alias, _)| alias);
    let incomplete_union_aliases = collected
        .union_aliases
        .iter()
        .filter(|(_, tag)| !unions.contains(*tag))
        .map(|(alias, _)| alias);
    for alias in incomplete_struct_aliases.chain(incomplete_union_aliases) {
        if contains_value_member(definition, alias) {
            return true;
        }
    }
    false
}

/// Whether `definition` uses `identifier` as a whole-word, by-value member type
/// (`identifier name;`) — i.e. not as a pointer (`identifier *name;`), which needs
/// no size. Used to spot by-value members of an incomplete typedef'd aggregate.
fn contains_value_member(definition: &str, identifier: &str) -> bool {
    let mut search_from = 0;
    while let Some(found) = definition[search_from..].find(identifier) {
        let start = search_from + found;
        let end = start + identifier.len();
        search_from = end;
        let before_ok = definition[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_c_identifier_char(ch));
        let after_ok = definition[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_c_identifier_char(ch));
        if !before_ok || !after_ok {
            continue;
        }
        // A pointer member (`alias *name`) is fine; only by-value members must be
        // sized.
        if !definition[end..].trim_start().starts_with('*') {
            return true;
        }
    }
    false
}

fn ordered_struct_definitions(collected: &Collected) -> Vec<&String> {
    fn visit<'a>(
        name: &str,
        collected: &'a Collected,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<&'a String>,
    ) {
        if visited.contains(name) || !visiting.insert(name.to_owned()) {
            return;
        }

        if let Some(definition) = collected.structs.get(name) {
            for dependency in struct_definition_dependencies(name, definition, collected) {
                visit(&dependency, collected, visiting, visited, ordered);
            }
            ordered.push(definition);
        }

        visiting.remove(name);
        visited.insert(name.to_owned());
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    for name in collected.structs.keys() {
        visit(name, collected, &mut visiting, &mut visited, &mut ordered);
    }
    ordered
}

fn struct_definition_dependencies(
    name: &str,
    definition: &str,
    collected: &Collected,
) -> Vec<String> {
    let mut dependencies = BTreeSet::new();
    for candidate in collected.structs.keys() {
        if candidate != name && contains_c_identifier(definition, candidate) {
            dependencies.insert(candidate.clone());
        }
    }
    for (alias, tag) in &collected.struct_aliases {
        if tag != name
            && collected.structs.contains_key(tag)
            && contains_c_identifier(definition, alias)
        {
            dependencies.insert(tag.clone());
        }
    }
    dependencies.into_iter().collect()
}

pub(super) fn contains_c_identifier(input: &str, identifier: &str) -> bool {
    let mut rest = input;
    while let Some(index) = rest.find(identifier) {
        let before = rest[..index].chars().next_back();
        let after = rest[index + identifier.len()..].chars().next();
        let boundary_before = before.is_none_or(|ch| !is_c_identifier_char(ch));
        let boundary_after = after.is_none_or(|ch| !is_c_identifier_char(ch));
        if boundary_before && boundary_after {
            return true;
        }
        rest = &rest[index + identifier.len()..];
    }
    false
}

pub(super) fn is_c_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Collect struct names that are referenced by the kept declarations but are
/// not defined here, so they can be forward-declared.
fn referenced_structs(
    typedefs: &[String],
    functions: &[String],
    defined: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut referenced = BTreeSet::new();
    for declaration in typedefs.iter().chain(functions.iter()) {
        let mut rest = declaration.as_str();
        while let Some(index) = rest.find("struct ") {
            rest = &rest[index + "struct ".len()..];
            let name = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect::<String>();
            if !name.is_empty() && name != "timeval" && !defined.contains_key(&name) {
                referenced.insert(name);
            }
        }
    }
    referenced
}

/// Replace `enum <name>` references (for enums projected onto `int`) with `int`.
/// Rename any parameter whose name shadows a declared type (typedef/struct/enum) to
/// `argN`. OpenBLAS's `cblas_cgemm_batch` has a parameter literally named `blasint`,
/// which is also a typedef — FFI's cdef parser then reads the name as a second type
/// and rejects the declaration. The name is cosmetic, so renaming keeps the cdef valid.
fn rename_type_colliding_params(function: &str, type_names: &BTreeSet<String>) -> String {
    let (Some(open), Some(close)) = (function.find('('), function.rfind(')')) else {
        return function.to_owned();
    };
    if close <= open {
        return function.to_owned();
    }
    let params = &function[open + 1..close];
    let renamed = params
        .split(',')
        .enumerate()
        .map(|(index, param)| rename_param_if_type(param, index, type_names))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}{renamed}{}", &function[..=open], &function[close..])
}

fn rename_param_if_type(param: &str, index: usize, type_names: &BTreeSet<String>) -> String {
    let trimmed = param.trim();
    let bytes = trimmed.as_bytes();
    let mut start = trimmed.len();
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    // Need a real type before the trailing identifier, separated from it.
    if start == 0 || start == trimmed.len() {
        return param.to_owned();
    }
    let prev = bytes[start - 1];
    if prev != b' ' && prev != b'*' && prev != b'\t' {
        return param.to_owned();
    }
    if !type_names.contains(&trimmed[start..]) {
        return param.to_owned();
    }
    let type_part = trimmed[..start].trim_end();
    // If everything before the trailing identifier is only cv-qualifiers, the
    // identifier IS an unnamed param's type (`const CBLAS_ORDER`, or `const int`
    // after enum projection), NOT a name — renaming it would destroy the type
    // (`const arg0`). Leave it alone.
    if type_part.is_empty() || type_part.split_whitespace().all(is_type_qualifier_keyword) {
        return param.to_owned();
    }
    let connector = if type_part.ends_with('*') { "" } else { " " };
    format!(" {type_part}{connector}arg{index}")
}

/// Project every `enum <Name>` *usage* to `int` (its FFI representation), so an
/// enum referenced by a struct field/parameter/return doesn't need its definition
/// in the cdef. This covers enums declared in a #included header that libclang only
/// forward-declares (FFmpeg's `AVMediaType`) and case-variant enum tags that would
/// otherwise generate colliding wrappers (OpenBLAS's `order`/`Order`). A definition
/// (`enum Name { ... }`) is left intact.
fn project_enums_to_int(input: &str, known_enums: &BTreeSet<String>) -> String {
    let bytes = input.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || !is_ident(bytes[i - 1]);
        if at_boundary && input[i..].starts_with("enum ") {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let id_start = j;
            while j < bytes.len() && is_ident(bytes[j]) {
                j += 1;
            }
            if j > id_start {
                let mut k = j;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                // `enum Name {` is a definition — keep it; a usage projects to the
                // bare typedef name when the enum is one this cdef emits as
                // `typedef int Name` (so it stays a PHP-enum-bearing name and still
                // resolves to int for FFI), otherwise to plain `int`.
                if !(k < bytes.len() && bytes[k] == b'{') {
                    let name = &input[id_start..j];
                    if known_enums.contains(name) {
                        out.extend_from_slice(name.as_bytes());
                    } else {
                        out.extend_from_slice(b"int");
                    }
                    i = j;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}
