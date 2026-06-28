//! C preprocessor macro handling: object/function-like macro expansion with
//! `##` token-paste and hide-sets, symbol/type aliases produced by fully-expanding
//! object-like macros, and rendering function-like macros into PHP free-function
//! bodies. Token helpers (`parse_call_args`/`substitute_params`) are shared with
//! `const_eval`.

use std::collections::{BTreeMap, BTreeSet};

use clang::token::TokenKind;

use super::*;

/// Parse a parenthesised, comma-separated argument list starting at the `(` at
/// `open`. Returns each argument's tokens and the index just past the matching
/// `)`, or `None` if the parentheses are unbalanced.
pub(super) fn parse_call_args(tokens: &[Token], open: usize) -> Option<(Vec<Vec<Token>>, usize)> {
    let mut depth = 0usize;
    let mut args: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    for (offset, token) in tokens[open..].iter().enumerate() {
        match token.1.as_str() {
            "(" => {
                depth += 1;
                if depth > 1 {
                    current.push(token.clone());
                }
            }
            ")" => {
                depth -= 1;
                if depth == 0 {
                    if !current.is_empty() || !args.is_empty() {
                        args.push(current);
                    }
                    return Some((args, open + offset + 1));
                }
                current.push(token.clone());
            }
            "," if depth == 1 => args.push(std::mem::take(&mut current)),
            _ => current.push(token.clone()),
        }
    }
    None
}

/// Replace each parameter identifier in a macro body with its argument's tokens.
pub(super) fn substitute_params(
    body: &[Token],
    params: &[String],
    args: &[Vec<Token>],
) -> Vec<Token> {
    let mut out = Vec::new();
    for token in body {
        if token.0 == TokenKind::Identifier
            && let Some(position) = params.iter().position(|param| *param == token.1)
        {
            out.extend(args[position].iter().cloned());
        } else {
            out.push(token.clone());
        }
    }
    out
}

/// Object-like macros that rename an export to a versioned symbol, e.g. ICU's
/// `#define u_errorName U_ICU_ENTRY_POINT_RENAME(u_errorName)` whose full
/// preprocessor expansion is the real symbol `u_errorName_74`. For each such
/// macro whose expansion is a single identifier naming a kept function, returns
/// `(public_name, native_symbol)` so the generated method can keep the friendly
/// name while dispatching to the versioned symbol. Fully generic: any
/// rename-via-macro scheme that token-pastes a version suffix is recovered, with
/// no hard-coded macro names.
pub(super) fn macro_symbol_aliases(
    collected: &Collected,
    definitions: &[crate::model::manifest::ResolvedDefinition],
) -> Vec<(String, String)> {
    // The resolved `require_definitions` as synthetic object-like macros, so an alias
    // macro that token-pastes the value (pcre2's `PCRE2_SUFFIX(name)`, which expands
    // through `PCRE2_CODE_UNIT_WIDTH`) reaches the real width-suffixed symbol — the
    // `-D` value is invisible to this simplified expander otherwise.
    let definition_tokens: Vec<(String, Vec<Token>)> = definitions
        .iter()
        .map(|definition| {
            (
                definition.name.clone(),
                vec![(TokenKind::Literal, definition.value.clone())],
            )
        })
        .collect();
    let mut object_macros: BTreeMap<String, &[Token]> = collected
        .macros
        .iter()
        .map(|macro_def| (macro_def.name.clone(), macro_def.tokens.as_slice()))
        .collect();
    for (name, tokens) in &definition_tokens {
        object_macros
            .entry(name.clone())
            .or_insert(tokens.as_slice());
    }
    let mut aliases = Vec::new();
    for macro_def in &collected.macros {
        // Seed the hide set with the macro's own name so a self-referential body
        // (`u_errorName` paste-base inside its own expansion) is left literal,
        // matching the C preprocessor's "painted blue" rule.
        let mut hide = BTreeSet::new();
        hide.insert(macro_def.name.clone());
        let Some(expanded) = expand_macro_seq(
            &macro_def.tokens,
            &object_macros,
            &collected.fn_macros,
            &hide,
            0,
        ) else {
            continue;
        };
        if let [(TokenKind::Identifier, symbol)] = expanded.as_slice()
            && symbol != &macro_def.name
            && collected.function_names.contains(symbol)
        {
            aliases.push((macro_def.name.clone(), symbol.clone()));
        }
    }
    aliases
}

/// Macro-expand a token sequence following object-like and function-like macros,
/// honouring `##` token paste, argument prescan, and a `hide` set that prevents a
/// macro from re-expanding inside its own replacement. A simplified Prosser
/// expansion: enough to recover symbol-version renames, not a full C preprocessor.
fn expand_macro_seq(
    tokens: &[Token],
    object_macros: &BTreeMap<String, &[Token]>,
    fn_macros: &BTreeMap<String, RawFnMacro>,
    hide: &BTreeSet<String>,
    depth: usize,
) -> Option<Vec<Token>> {
    if depth > MAX_MACRO_EXPANSION_DEPTH {
        return None;
    }
    let mut out: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (kind, spelling) = &tokens[index];
        if *kind == TokenKind::Identifier && !hide.contains(spelling) {
            if let Some(macro_def) = fn_macros.get(spelling)
                && tokens.get(index + 1).is_some_and(|(_, next)| next == "(")
            {
                let (args, after) = parse_call_args(tokens, index + 1)?;
                if args.len() != macro_def.params.len() {
                    return None;
                }
                // Prescan: each argument is fully expanded before substitution,
                // except where it is a `##` operand (handled token-by-token below).
                let expanded_args = args
                    .iter()
                    .map(|arg| expand_macro_seq(arg, object_macros, fn_macros, hide, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                let substituted = substitute_with_paste(
                    &macro_def.body,
                    &macro_def.params,
                    &args,
                    &expanded_args,
                );
                let mut inner_hide = hide.clone();
                inner_hide.insert(spelling.clone());
                let rescanned = expand_macro_seq(
                    &substituted,
                    object_macros,
                    fn_macros,
                    &inner_hide,
                    depth + 1,
                )?;
                out.extend(rescanned);
                index = after;
                continue;
            }
            if let Some(body) = object_macros.get(spelling) {
                let mut inner_hide = hide.clone();
                inner_hide.insert(spelling.clone());
                let pasted = apply_token_paste(body);
                let rescanned =
                    expand_macro_seq(&pasted, object_macros, fn_macros, &inner_hide, depth + 1)?;
                out.extend(rescanned);
                index += 1;
                continue;
            }
        }
        out.push(tokens[index].clone());
        index += 1;
    }
    Some(out)
}

/// Substitute a function-like macro's arguments into its body, applying the `##`
/// token-paste operator. Paste operands use the *raw* (unexpanded) argument, every
/// other parameter use its prescan-expanded form, matching the preprocessor.
fn substitute_with_paste(
    body: &[Token],
    params: &[String],
    raw_args: &[Vec<Token>],
    expanded_args: &[Vec<Token>],
) -> Vec<Token> {
    // Resolve a body token to the tokens it stands for, using `raw` argument
    // tokens for paste operands and expanded ones elsewhere.
    let resolve = |token: &Token, raw: bool| -> Vec<Token> {
        if token.0 == TokenKind::Identifier
            && let Some(position) = params.iter().position(|param| param == &token.1)
        {
            let source = if raw { raw_args } else { expanded_args };
            return source[position].clone();
        }
        vec![token.clone()]
    };

    let mut out: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let is_paste = body.get(index + 1).is_some_and(|(_, next)| next == "##");
        if is_paste {
            // Collect a `a ## b ## c` run, pasting each raw operand's spelling.
            let mut pasted = token_spellings(&resolve(&body[index], true));
            index += 1;
            while body
                .get(index)
                .is_some_and(|(_, spelling)| spelling == "##")
            {
                let operand = body.get(index + 1).cloned();
                index += 2;
                if let Some(operand) = operand {
                    pasted.push_str(&token_spellings(&resolve(&operand, true)));
                }
            }
            out.push((TokenKind::Identifier, pasted));
        } else {
            out.extend(resolve(&body[index], false));
            index += 1;
        }
    }
    out
}

/// Apply `##` token paste within an object-like macro body (no parameters).
fn apply_token_paste(body: &[Token]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < body.len() {
        if body.get(index + 1).is_some_and(|(_, next)| next == "##") {
            let mut pasted = body[index].1.clone();
            index += 1;
            while body
                .get(index)
                .is_some_and(|(_, spelling)| spelling == "##")
            {
                if let Some(operand) = body.get(index + 1) {
                    pasted.push_str(&operand.1);
                }
                index += 2;
            }
            out.push((TokenKind::Identifier, pasted));
        } else {
            out.push(body[index].clone());
            index += 1;
        }
    }
    out
}

/// Concatenate the spellings of a token run into one identifier fragment.
fn token_spellings(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|(_, spelling)| spelling.as_str())
        .collect()
}

/// PHP-compatible rendering of a single C literal token (`0x20`, `1u`, `1.5f`,
/// `"text"`). Char literals and anything unrecognised yield `None`.
fn translate_literal(literal: &str) -> Option<String> {
    match literal.chars().next()? {
        '"' => Some(literal.to_owned()),
        '\'' => None,
        _ => translate_number(literal),
    }
}

/// Strip C integer/float suffixes and validate the remaining numeric literal so
/// PHP reads the same value. Hex/binary keep their `0x`/`0b` prefix; the `f`/`F`
/// hex digits are never mistaken for a float suffix.
fn translate_number(literal: &str) -> Option<String> {
    let lower = literal.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        // Validate on the lowercased form but keep the original digit case.
        let digits = hex.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| format!("0x{}", &literal[2..2 + digits.len()]));
    }
    if let Some(binary) = lower.strip_prefix("0b") {
        let digits = binary.trim_end_matches(['u', 'l']);
        return (!digits.is_empty() && digits.bytes().all(|b| b == b'0' || b == b'1'))
            .then(|| format!("0b{}", &literal[2..2 + digits.len()]));
    }
    let trimmed = lower.trim_end_matches(['u', 'l', 'f']);
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'+' | b'-'))
        // A C pp-number allows several dots (`1.1.0`, a version literal), but that
        // is not a valid PHP number — reject it instead of emitting `const X = 1.1.0;`.
        || trimmed.bytes().filter(|b| *b == b'.').count() > 1
        || trimmed.bytes().filter(|b| *b == b'e').count() > 1
    {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Operators safe to pass through verbatim into a PHP constant expression (they
/// have the same meaning and precedence as in C).
fn is_allowed_operator(token: &str) -> bool {
    matches!(
        token,
        "(" | ")" | "+" | "-" | "*" | "/" | "%" | "<<" | ">>" | "&" | "|" | "^" | "~"
    )
}

/// Why a function-like macro body could not be rendered as a PHP expression.
enum FnBodyError {
    /// It calls a C function that is not in this library (named here).
    UndefinedCall(String),
    /// It uses something with no PHP equivalent (cast, char literal, unknown name).
    Untranslatable,
}

/// Turn each prefix-matching function-like macro into a [`MacroFunction`].
///
/// A macro whose body renders cleanly becomes a `return <expr>;` function; one
/// that calls a C function this library does not define becomes a throwing
/// function; anything untranslatable is dropped.
pub(super) fn macro_functions(
    collected: &Collected,
    prefix: &str,
    consts: &BTreeSet<String>,
    options: &HeaderAdapterOptions,
) -> Vec<MacroFunction> {
    let needle = prefix.to_ascii_lowercase();
    let params_per_macro: BTreeMap<&String, BTreeSet<&str>> = collected
        .fn_macros
        .iter()
        .map(|(name, macro_def)| (name, macro_def.params.iter().map(String::as_str).collect()))
        .collect();

    let mut functions = Vec::new();
    for (name, macro_def) in &collected.fn_macros {
        if !name.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        let context = FnBodyContext {
            params: &params_per_macro[name],
            consts,
            functions: &collected.function_names,
            fn_macros: &collected.fn_macros,
            entity_fqcn: &options.entity_fqcn,
            // Constants live in the entity's namespace (parent of the entity class).
            const_namespace: options
                .entity_fqcn
                .rsplit_once('\\')
                .map_or(options.entity_fqcn.as_str(), |(namespace, _)| namespace),
            dependency_functions: &options.dependency_functions,
        };
        let body = match render_fn_body(&macro_def.body, &context, 0) {
            Ok(expr) => Ok(expr),
            Err(FnBodyError::UndefinedCall(symbol)) => Err(symbol),
            Err(FnBodyError::Untranslatable) => continue,
        };
        functions.push(MacroFunction {
            name: name.clone(),
            params: macro_def.params.clone(),
            body,
        });
    }
    functions
}

/// Everything `render_fn_body` needs to resolve the identifiers in a macro body.
struct FnBodyContext<'a> {
    params: &'a BTreeSet<&'a str>,
    consts: &'a BTreeSet<String>,
    functions: &'a BTreeSet<String>,
    fn_macros: &'a BTreeMap<String, RawFnMacro>,
    entity_fqcn: &'a str,
    const_namespace: &'a str,
    dependency_functions: &'a BTreeMap<String, String>,
}

/// Render a function-like macro body as a PHP expression: parameters become
/// `$param`, this library's C functions become `<Class>::fn(...)` static calls
/// (a dependency's function calls the dependency's class), known constants become
/// fully-qualified references, and a nested function-like macro call is expanded
/// inline. A call to a C function nobody defines yields `UndefinedCall`.
fn render_fn_body(
    tokens: &[Token],
    context: &FnBodyContext<'_>,
    depth: usize,
) -> std::result::Result<String, FnBodyError> {
    if depth > MAX_MACRO_EXPANSION_DEPTH {
        return Err(FnBodyError::Untranslatable);
    }
    let mut parts = Vec::new();
    // Distinguishes a prefix `&` (C address-of, which has no faithful PHP
    // rendering) from a binary `&` (bitwise-and): a prefix operator has no
    // operand before it. `)` closes a parenthesised operand. Also drives PHP `.`
    // insertion between adjacent operands (C string-literal concatenation, e.g.
    // duktape's `"\x80" x`).
    let mut prev_was_operand = false;
    let mut has_operand = false;
    let mut index = 0;
    while index < tokens.len() {
        let (kind, spelling) = &tokens[index];
        let is_call = tokens.get(index + 1).is_some_and(|(_, next)| next == "(");
        match kind {
            TokenKind::Literal => {
                let literal = translate_literal(spelling).ok_or(FnBodyError::Untranslatable)?;
                if prev_was_operand {
                    parts.push(".".to_owned());
                }
                parts.push(literal);
                prev_was_operand = true;
                has_operand = true;
                index += 1;
            }
            // A prefix `&`/`*` (address-of / dereference) can't be expressed as a
            // PHP call argument, so the whole macro is dropped rather than emitting
            // invalid PHP like `fn(& ( $x ))`.
            TokenKind::Punctuation
                if matches!(spelling.as_str(), "&" | "*") && !prev_was_operand =>
            {
                return Err(FnBodyError::Untranslatable);
            }
            // A `*`/`&` immediately before `)` is a pointer marker in a C cast
            // (`(type *)`), not multiplication — that whole cast idiom (glib's
            // `g_chunk_new` → `((type *) g_mem_chunk_alloc(...))`) has no PHP form.
            TokenKind::Punctuation
                if matches!(spelling.as_str(), "&" | "*")
                    && tokens.get(index + 1).is_some_and(|(_, next)| next == ")") =>
            {
                return Err(FnBodyError::Untranslatable);
            }
            TokenKind::Punctuation if is_allowed_operator(spelling) => {
                prev_was_operand = spelling == ")";
                parts.push(spelling.clone());
                index += 1;
            }
            TokenKind::Identifier if is_call => {
                let (args, after) =
                    parse_call_args(tokens, index + 1).ok_or(FnBodyError::Untranslatable)?;
                if prev_was_operand {
                    parts.push(".".to_owned());
                }
                let render_args = || {
                    args.iter()
                        .map(|arg| render_fn_body(arg, context, depth + 1))
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map(|rendered| rendered.join(", "))
                };
                if let Some(macro_def) = context.fn_macros.get(spelling) {
                    if args.len() != macro_def.params.len() {
                        return Err(FnBodyError::Untranslatable);
                    }
                    let substituted = substitute_params(&macro_def.body, &macro_def.params, &args);
                    let expanded = render_fn_body(&substituted, context, depth + 1)?;
                    // A nested call expanding to nothing would emit `()`, not a PHP
                    // expression — drop the whole macro function.
                    if expanded.is_empty() {
                        return Err(FnBodyError::Untranslatable);
                    }
                    parts.push(format!("({expanded})"));
                } else if context.functions.contains(spelling) {
                    parts.push(format!(
                        "{}::{spelling}({})",
                        context.entity_fqcn,
                        render_args()?
                    ));
                } else if let Some(dep_fqcn) = context.dependency_functions.get(spelling) {
                    parts.push(format!("{dep_fqcn}::{spelling}({})", render_args()?));
                } else {
                    return Err(FnBodyError::UndefinedCall(spelling.clone()));
                }
                prev_was_operand = true;
                has_operand = true;
                index = after;
            }
            TokenKind::Identifier if context.params.contains(spelling.as_str()) => {
                if prev_was_operand {
                    parts.push(".".to_owned());
                }
                parts.push(format!("${spelling}"));
                prev_was_operand = true;
                has_operand = true;
                index += 1;
            }
            TokenKind::Identifier if context.consts.contains(spelling) => {
                if prev_was_operand {
                    parts.push(".".to_owned());
                }
                parts.push(format!("{}\\{spelling}", context.const_namespace));
                prev_was_operand = true;
                has_operand = true;
                index += 1;
            }
            _ => return Err(FnBodyError::Untranslatable),
        }
    }
    // A body with no operand at all (e.g. lzo2's `LZO_PP_ECONCAT0()` → `()`) has
    // no PHP expression to return.
    if !has_operand {
        return Err(FnBodyError::Untranslatable);
    }
    Ok(parts.join(" "))
}
