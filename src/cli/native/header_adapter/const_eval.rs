//! C constant-expression evaluation: parse a macro/enum constant's tokens into a
//! typed [`ConstValue`] (C11 6.4.4.1 integer classification, float/char/string
//! literals, the usual operators), then render it to the PHP wrapper + scalar
//! forms. Used by `header_constants` in the parent module.

use std::collections::BTreeMap;

use clang::token::TokenKind;

use super::macros::{parse_call_args, substitute_params};
use super::{MAX_MACRO_EXPANSION_DEPTH, RawFnMacro, Token};

/// The C type of an integer constant (C11 6.4.4.1), on a target where `int` is
/// 32-bit and `long`/`long long` are 64-bit. Drives the wrapper class and the
/// scalar-variant decision.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IntKind {
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
}

impl IntKind {
    fn unsigned(self) -> bool {
        matches!(self, IntKind::UInt | IntKind::ULong | IntKind::ULongLong)
    }

    fn rank(self) -> u8 {
        match self {
            IntKind::Int | IntKind::UInt => 0,
            IntKind::Long | IntKind::ULong => 1,
            IntKind::LongLong | IntKind::ULongLong => 2,
        }
    }

    fn of(rank: u8, unsigned: bool) -> IntKind {
        match (rank, unsigned) {
            (0, false) => IntKind::Int,
            (0, true) => IntKind::UInt,
            (1, false) => IntKind::Long,
            (1, true) => IntKind::ULong,
            (_, false) => IntKind::LongLong,
            (_, true) => IntKind::ULongLong,
        }
    }

    /// The result type of a binary integer operation, per the usual arithmetic
    /// conversions (highest rank wins; unsigned is contagious).
    fn combine(self, other: IntKind) -> IntKind {
        IntKind::of(
            self.rank().max(other.rank()),
            self.unsigned() || other.unsigned(),
        )
    }

    fn wrapper(self) -> &'static str {
        match self {
            IntKind::Int => "Int_",
            IntKind::UInt => "UnsignedInt",
            IntKind::Long => "Long",
            IntKind::ULong => "UnsignedLong",
            IntKind::LongLong => "LongLong",
            IntKind::ULongLong => "UnsignedLongLong",
        }
    }
}

/// The C type of a floating constant: `double` by default, `float` for an `f`
/// suffix, `long double` for an `l` suffix.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FloatKind {
    Float,
    Double,
    LongDouble,
}

impl FloatKind {
    fn rank(self) -> u8 {
        match self {
            FloatKind::Float => 0,
            FloatKind::Double => 1,
            FloatKind::LongDouble => 2,
        }
    }

    fn combine(self, other: FloatKind) -> FloatKind {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }

    fn wrapper(self) -> &'static str {
        match self {
            FloatKind::Float => "Float_",
            FloatKind::Double => "Double",
            FloatKind::LongDouble => "LongDouble",
        }
    }
}

/// A C constant expression evaluated to a typed value. The integer value is held
/// in an `i128`, wide enough for the full `unsigned long long` range. `Str` holds
/// a ready PHP string expression (one `"..."` literal, or several joined with `.`
/// for C's adjacent-string concatenation).
#[derive(Clone)]
pub(super) enum ConstValue {
    Int(i128, IntKind),
    Float(f64, FloatKind),
    Str(String),
}

/// Evaluate a macro replacement's tokens to a typed constant, resolving
/// references to already-known constants (`env`) and expanding constant-argument
/// calls to function-like macros. Returns `None` for anything that is not a fully
/// resolvable constant expression (casts, `sizeof`, unknown identifiers, calls to
/// real C functions), so the constant is dropped rather than mis-rendered.
pub(super) fn eval_const(
    tokens: &[Token],
    env: &BTreeMap<String, ConstValue>,
    fn_macros: &BTreeMap<String, RawFnMacro>,
    depth: usize,
) -> Option<ConstValue> {
    if depth > MAX_MACRO_EXPANSION_DEPTH {
        return None;
    }
    let mut parser = ConstParser {
        tokens,
        pos: 0,
        env,
        fn_macros,
        depth,
    };
    let value = parser.parse_ternary()?;
    // A leftover token means the body is not a single constant expression (a stray
    // cast, attribute, or trailing call) — drop it rather than emit a prefix of it.
    if parser.pos != tokens.len() {
        return None;
    }
    Some(value)
}

/// A recursive-descent (precedence-climbing) parser over a macro body's tokens.
struct ConstParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    env: &'a BTreeMap<String, ConstValue>,
    fn_macros: &'a BTreeMap<String, RawFnMacro>,
    depth: usize,
}

impl ConstParser<'_> {
    fn peek_spelling(&self) -> Option<&str> {
        self.tokens
            .get(self.pos)
            .map(|(_, spelling)| spelling.as_str())
    }

    fn parse_ternary(&mut self) -> Option<ConstValue> {
        let condition = self.parse_binary(1)?;
        if self.peek_spelling() == Some("?") {
            self.pos += 1;
            let then_value = self.parse_ternary()?;
            if self.peek_spelling() != Some(":") {
                return None;
            }
            self.pos += 1;
            let else_value = self.parse_ternary()?;
            return Some(if const_truthy(&condition)? {
                then_value
            } else {
                else_value
            });
        }
        Some(condition)
    }

    fn parse_binary(&mut self, min_bp: u8) -> Option<ConstValue> {
        let mut lhs = self.parse_unary()?;
        while let Some(bp) = self.peek_spelling().and_then(binary_binding_power) {
            if bp < min_bp {
                break;
            }
            let op = self.peek_spelling()?.to_owned();
            self.pos += 1;
            let rhs = self.parse_binary(bp + 1)?;
            lhs = apply_binary(&op, lhs, rhs)?;
        }
        Some(lhs)
    }

    fn parse_unary(&mut self) -> Option<ConstValue> {
        match self.peek_spelling() {
            Some(op @ ("+" | "-" | "~" | "!")) => {
                let op = op.to_owned();
                self.pos += 1;
                let operand = self.parse_unary()?;
                apply_unary(&op, operand)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Option<ConstValue> {
        let (kind, spelling) = self.tokens.get(self.pos)?.clone();
        match kind {
            TokenKind::Punctuation if spelling == "(" => {
                self.pos += 1;
                let inner = self.parse_ternary()?;
                if self.peek_spelling() != Some(")") {
                    return None;
                }
                self.pos += 1;
                Some(inner)
            }
            TokenKind::Literal if spelling.starts_with('"') => self.parse_string(),
            TokenKind::Literal if spelling.starts_with('\'') => {
                self.pos += 1;
                parse_char_literal(&spelling)
            }
            TokenKind::Literal => {
                self.pos += 1;
                parse_number_literal(&spelling)
            }
            // A function-like macro called with constant arguments: substitute and
            // evaluate its body (`EX_POS_DISPLAY(0)` -> `(EX_POS_MASK | (0))`).
            TokenKind::Identifier
                if self
                    .tokens
                    .get(self.pos + 1)
                    .is_some_and(|(_, next)| next == "(") =>
            {
                let macro_def = self.fn_macros.get(&spelling)?;
                let (args, after) = parse_call_args(self.tokens, self.pos + 1)?;
                if args.len() != macro_def.params.len() {
                    return None;
                }
                let substituted = substitute_params(&macro_def.body, &macro_def.params, &args);
                let value = eval_const(&substituted, self.env, self.fn_macros, self.depth + 1)?;
                self.pos = after;
                Some(value)
            }
            TokenKind::Identifier => {
                let value = self.env.get(&spelling)?.clone();
                self.pos += 1;
                Some(value)
            }
            _ => None,
        }
    }

    /// A string literal, joining C-adjacent string literals (and string-valued
    /// constant references) with PHP's `.` operator.
    fn parse_string(&mut self) -> Option<ConstValue> {
        let mut parts = Vec::new();
        while let Some((kind, spelling)) = self.tokens.get(self.pos) {
            match kind {
                TokenKind::Literal if spelling.starts_with('"') => {
                    // A C string literal is emitted as a PHP double-quoted string
                    // (the `\n`/`\t`/`\"` escapes match), but PHP also interpolates
                    // `$name`/`{$...}`. A literal `$` in the C string (assimp's
                    // `"$tex.file"`) must be escaped, or the generated `const`
                    // becomes "invalid operations in a constant expression".
                    parts.push(spelling.replace('$', "\\$"));
                    self.pos += 1;
                }
                TokenKind::Identifier => match self.env.get(spelling) {
                    Some(ConstValue::Str(expr)) => {
                        parts.push(expr.clone());
                        self.pos += 1;
                    }
                    _ => break,
                },
                _ => break,
            }
        }
        if parts.is_empty() {
            return None;
        }
        Some(ConstValue::Str(parts.join(" . ")))
    }
}

fn const_truthy(value: &ConstValue) -> Option<bool> {
    match value {
        ConstValue::Int(v, _) => Some(*v != 0),
        ConstValue::Float(v, _) => Some(*v != 0.0),
        ConstValue::Str(_) => None,
    }
}

/// Left binding power of a binary operator (higher binds tighter); `None` for a
/// token that is not a constant-expression binary operator.
fn binary_binding_power(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | "<=" | ">" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => return None,
    })
}

fn apply_unary(op: &str, operand: ConstValue) -> Option<ConstValue> {
    match (op, operand) {
        ("+", value) => Some(value),
        ("-", ConstValue::Int(v, k)) => Some(ConstValue::Int(v.checked_neg()?, k)),
        ("-", ConstValue::Float(v, k)) => Some(ConstValue::Float(-v, k)),
        ("~", ConstValue::Int(v, k)) => Some(ConstValue::Int(!v, k)),
        ("!", ConstValue::Int(v, _)) => Some(ConstValue::Int((v == 0) as i128, IntKind::Int)),
        ("!", ConstValue::Float(v, _)) => Some(ConstValue::Int((v == 0.0) as i128, IntKind::Int)),
        _ => None,
    }
}

fn apply_binary(op: &str, lhs: ConstValue, rhs: ConstValue) -> Option<ConstValue> {
    match (lhs, rhs) {
        (ConstValue::Int(x, kx), ConstValue::Int(y, ky)) => apply_int_binary(op, x, kx, y, ky),
        (ConstValue::Float(x, kx), ConstValue::Float(y, ky)) => {
            apply_float_binary(op, x, y, kx.combine(ky))
        }
        (ConstValue::Float(x, kx), ConstValue::Int(y, _)) => {
            apply_float_binary(op, x, y as f64, kx)
        }
        (ConstValue::Int(x, _), ConstValue::Float(y, ky)) => {
            apply_float_binary(op, x as f64, y, ky)
        }
        // Strings only combine by juxtaposition (handled in parse_string); any
        // operator on a string is not a constant we can render.
        _ => None,
    }
}

fn apply_int_binary(op: &str, x: i128, kx: IntKind, y: i128, ky: IntKind) -> Option<ConstValue> {
    let kind = kx.combine(ky);
    let arith = |v: i128| Some(ConstValue::Int(v, kind));
    let logic = |b: bool| Some(ConstValue::Int(b as i128, IntKind::Int));
    match op {
        "+" => arith(x.checked_add(y)?),
        "-" => arith(x.checked_sub(y)?),
        "*" => arith(x.checked_mul(y)?),
        "/" => (y != 0).then(|| ConstValue::Int(x / y, kind)),
        "%" => (y != 0).then(|| ConstValue::Int(x % y, kind)),
        "<<" => arith(x.checked_shl(u32::try_from(y).ok()?)?),
        ">>" => arith(x.checked_shr(u32::try_from(y).ok()?)?),
        "&" => arith(x & y),
        "|" => arith(x | y),
        "^" => arith(x ^ y),
        "<" => logic(x < y),
        "<=" => logic(x <= y),
        ">" => logic(x > y),
        ">=" => logic(x >= y),
        "==" => logic(x == y),
        "!=" => logic(x != y),
        "&&" => logic(x != 0 && y != 0),
        "||" => logic(x != 0 || y != 0),
        _ => None,
    }
}

fn apply_float_binary(op: &str, x: f64, y: f64, kind: FloatKind) -> Option<ConstValue> {
    let logic = |b: bool| Some(ConstValue::Int(b as i128, IntKind::Int));
    match op {
        "+" => Some(ConstValue::Float(x + y, kind)),
        "-" => Some(ConstValue::Float(x - y, kind)),
        "*" => Some(ConstValue::Float(x * y, kind)),
        "/" => Some(ConstValue::Float(x / y, kind)),
        "<" => logic(x < y),
        "<=" => logic(x <= y),
        ">" => logic(x > y),
        ">=" => logic(x >= y),
        "==" => logic(x == y),
        "!=" => logic(x != y),
        "&&" => logic(x != 0.0 && y != 0.0),
        "||" => logic(x != 0.0 || y != 0.0),
        // %, shifts and bitwise ops are not defined on floating operands.
        _ => None,
    }
}

/// A C character constant (`'A'`, `'\n'`) as its integer value; a multi-character
/// or unrecognised-escape constant yields `None` (the constant is dropped).
fn parse_char_literal(literal: &str) -> Option<ConstValue> {
    let inner = literal.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut chars = inner.chars();
    let value: i128 = match chars.next()? {
        '\\' => match chars.next()? {
            'n' => 10,
            't' => 9,
            'r' => 13,
            '0' => 0,
            '\\' => 92,
            '\'' => 39,
            '"' => 34,
            'a' => 7,
            'b' => 8,
            'f' => 12,
            'v' => 11,
            _ => return None,
        },
        ch if ch.is_ascii() => ch as i128,
        _ => return None,
    };
    // Reject a multi-character constant ('ab'), whose value is implementation-defined.
    if chars.next().is_some() {
        return None;
    }
    Some(ConstValue::Int(value, IntKind::Int))
}

/// A C numeric literal (`42`, `0x20`, `0b1010`, `3.14f`, `1e9`) as a typed value.
fn parse_number_literal(literal: &str) -> Option<ConstValue> {
    let lower = literal.to_ascii_lowercase();
    if lower.starts_with("0x") || lower.starts_with("0b") {
        return parse_int_literal(&lower);
    }
    // A decimal literal with a fraction or exponent is a floating constant.
    let is_float = lower.contains('.') || (lower.contains('e') && !lower.ends_with('e'));
    if is_float {
        parse_float_literal(&lower)
    } else {
        parse_int_literal(&lower)
    }
}

/// Parse a (lowercased) integer literal, choosing its C type from the suffix and
/// magnitude. Returns `None` for a malformed literal (e.g. `1.2.3`).
fn parse_int_literal(lower: &str) -> Option<ConstValue> {
    let digits_end = lower
        .rfind(|ch: char| ch != 'u' && ch != 'l')
        .map(|index| index + 1)
        .unwrap_or(0);
    let (body, suffix) = lower.split_at(digits_end);
    if suffix.chars().any(|ch| ch != 'u' && ch != 'l') {
        return None;
    }
    let has_u = suffix.contains('u');
    let l_count = suffix.matches('l').count().min(2) as u8;

    let (radix, digits, decimal) = if let Some(hex) = body.strip_prefix("0x") {
        (16u32, hex, false)
    } else if let Some(binary) = body.strip_prefix("0b") {
        (2u32, binary, false)
    } else if body.len() > 1 && body.starts_with('0') {
        (8u32, &body[1..], false)
    } else {
        (10u32, body, true)
    };
    if digits.is_empty() {
        return None;
    }
    let value = u128::from_str_radix(digits, radix).ok()?;
    let kind = classify_int(value, decimal, has_u, l_count);
    Some(ConstValue::Int(value as i128, kind))
}

/// The C type of an integer constant (C11 6.4.4.1): the first type in the
/// suffix's candidate list whose range holds `value`, on a 32-bit-`int` /
/// 64-bit-`long` target.
fn classify_int(value: u128, decimal: bool, has_u: bool, l_count: u8) -> IntKind {
    const I32_MAX: u128 = i32::MAX as u128;
    const U32_MAX: u128 = u32::MAX as u128;
    const I64_MAX: u128 = i64::MAX as u128;
    const U64_MAX: u128 = u64::MAX as u128;
    let signed_max = |rank: u8| if rank == 0 { I32_MAX } else { I64_MAX };
    let unsigned_max = |rank: u8| if rank == 0 { U32_MAX } else { U64_MAX };
    for rank in l_count..=2 {
        if !has_u && value <= signed_max(rank) {
            return IntKind::of(rank, false);
        }
        // A decimal constant without a `u` suffix only becomes unsigned if it
        // overflows every signed type; hex/octal may pick an unsigned type sooner.
        if (has_u || !decimal) && value <= unsigned_max(rank) {
            return IntKind::of(rank, true);
        }
    }
    IntKind::ULongLong
}

/// Parse a (lowercased) floating literal, choosing `double`/`float`/`long double`
/// from its suffix.
fn parse_float_literal(lower: &str) -> Option<ConstValue> {
    let (body, kind) = if let Some(rest) = lower.strip_suffix('f') {
        (rest, FloatKind::Float)
    } else if let Some(rest) = lower.strip_suffix('l') {
        (rest, FloatKind::LongDouble)
    } else {
        (lower, FloatKind::Double)
    };
    // C permits a trailing dot (`1.`), which Rust's float parser rejects.
    let body = if body.ends_with('.') {
        format!("{body}0")
    } else {
        body.to_owned()
    };
    let value: f64 = body.parse().ok()?;
    Some(ConstValue::Float(value, kind))
}

/// Render a constant's value as the two forms `const.php` needs: the wrapped
/// `\Pnlx\Types\*` object (default), and the bare PHP scalar for the `scalar/`
/// variant — used only for a plain `int`/`double`/string, with typed and unsigned
/// values staying wrapped (mirroring `use_php_scalars_in_return`).
pub(super) fn render_const_value(value: &ConstValue) -> (String, String) {
    match value {
        ConstValue::Int(v, kind) => {
            let wrapped = format!(
                "new \\Pnlx\\Types\\{}({})",
                kind.wrapper(),
                int_constructor_arg(*v)
            );
            let scalar = if *kind == IntKind::Int {
                v.to_string()
            } else {
                wrapped.clone()
            };
            (wrapped, scalar)
        }
        ConstValue::Float(v, kind) => {
            let literal = float_literal_text(*v);
            let wrapped = format!("new \\Pnlx\\Types\\{}({literal})", kind.wrapper());
            let scalar = if *kind == FloatKind::Double {
                literal
            } else {
                wrapped.clone()
            };
            (wrapped, scalar)
        }
        ConstValue::Str(expr) => (format!("new \\Pnlx\\Types\\String_({expr})"), expr.clone()),
    }
}

/// The PHP literal handed to an integer wrapper's constructor: a plain `int` when
/// the value fits PHP's signed 64-bit `int`, otherwise a decimal string the
/// wrapper folds back to a 64-bit pattern (so an `unsigned long long` above
/// `PHP_INT_MAX` survives losslessly).
fn int_constructor_arg(value: i128) -> String {
    if value == i64::MIN as i128 {
        // PHP parses the literal `-9223372036854775808` by first reading the
        // magnitude, which overflows `PHP_INT_MAX` and becomes a float; and the
        // string form trips the wrapper's fold (negating the magnitude overflows
        // too). `PHP_INT_MIN` is exactly `i64::MIN`, so emit it as an int expression.
        "(-9223372036854775807 - 1)".to_owned()
    } else if value >= i64::MIN as i128 && value <= i64::MAX as i128 {
        value.to_string()
    } else {
        format!("'{value}'")
    }
}

/// A PHP float literal that always reads back as a float (Rust's `{:?}` keeps a
/// `.0` on whole numbers and uses `e` notation where needed).
fn float_literal_text(value: f64) -> String {
    // Rust's `{:?}` renders non-finite floats as `inf`/`-inf`/`NaN`, none of which
    // are valid PHP — PHP spells them with the predefined constants `INF`/`NAN`. A
    // macro that evaluates to one (duktape's `DUK_DOUBLE_INFINITY = (1.0 / 0.0)`)
    // would otherwise emit `const … = inf;` → "Undefined constant".
    if value.is_nan() {
        "NAN".to_owned()
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            "-INF".to_owned()
        } else {
            "INF".to_owned()
        }
    } else {
        format!("{value:?}")
    }
}

/// A resolved `require_definitions` value as a typed constant, by its declaration
/// (an `int` width is an `int`, a `string` a string). `None` if the recorded value
/// does not parse for its type (it was validated at install, so this is defensive).
pub(super) fn definition_const_value(
    definition: &crate::model::manifest::ResolvedDefinition,
) -> Option<ConstValue> {
    use crate::model::manifest::DefinitionType;
    Some(match definition.definition_type {
        DefinitionType::Int => ConstValue::Int(definition.value.parse().ok()?, IntKind::Int),
        DefinitionType::Float => {
            ConstValue::Float(definition.value.parse().ok()?, FloatKind::Double)
        }
        // A preprocessor boolean is the `int` 0/1 it expands to.
        DefinitionType::Boolean => {
            ConstValue::Int(i128::from(definition.value == "1"), IntKind::Int)
        }
        DefinitionType::String => ConstValue::Str(format!(
            "\"{}\"",
            definition
                .value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
        )),
    })
}
