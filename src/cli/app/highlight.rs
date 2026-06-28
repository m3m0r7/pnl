//! A tiny, dependency-free PHP highlighter for terminal output.
//!
//! It is a single-line lexer used to colourise the ```php code blocks shown by
//! `pnl install` (see [`crate::app::ui::example_block`]). It is deliberately modest:
//! it recognises comments, strings, variables, numbers, and keywords, which is
//! enough to make a usage snippet readable without pulling in a real grammar.
//! Styling is delegated to [`crate::app::ui`], so it is automatically disabled when
//! output is not a TTY or `NO_COLOR` is set.

/// PHP keywords worth emphasising in a short usage snippet.
const KEYWORDS: &[&str] = &[
    "abstract",
    "and",
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
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
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
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

/// Literal-ish words rendered like values rather than keywords.
const LITERALS: &[&str] = &[
    "true", "false", "null", "TRUE", "FALSE", "NULL", "self", "parent",
];

/// Highlight a single line of PHP source for the terminal.
pub fn php_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            // Line comments run to the end of the line.
            '/' if chars.get(index + 1) == Some(&'/') => {
                out.push_str(&crate::app::ui::dim(
                    &chars[index..].iter().collect::<String>(),
                ));
                break;
            }
            '#' => {
                out.push_str(&crate::app::ui::dim(
                    &chars[index..].iter().collect::<String>(),
                ));
                break;
            }
            // Block comments: highlight to the closing `*/` on this line, else EOL.
            '/' if chars.get(index + 1) == Some(&'*') => {
                let start = index;
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                if index < chars.len() {
                    index += 2; // consume the closing `*/`
                }
                out.push_str(&crate::app::ui::dim(
                    &chars[start..index].iter().collect::<String>(),
                ));
            }
            // Single- and double-quoted strings (with backslash escapes).
            '\'' | '"' => {
                let quote = ch;
                let start = index;
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == quote {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                out.push_str(&crate::app::ui::green(
                    &chars[start..index].iter().collect::<String>(),
                ));
            }
            // Variables: `$name`.
            '$' => {
                let start = index;
                index += 1;
                while index < chars.len() && is_ident(chars[index]) {
                    index += 1;
                }
                out.push_str(&crate::app::ui::cyan(
                    &chars[start..index].iter().collect::<String>(),
                ));
            }
            // Numbers, including `0x..` hex.
            c if c.is_ascii_digit() => {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric()
                        || chars[index] == '.'
                        || chars[index] == '_')
                {
                    index += 1;
                }
                out.push_str(&crate::app::ui::yellow(
                    &chars[start..index].iter().collect::<String>(),
                ));
            }
            // Bare identifiers: keyword, literal, or plain text.
            c if is_ident_start(c) => {
                let start = index;
                while index < chars.len() && is_ident(chars[index]) {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                if KEYWORDS.contains(&word.as_str()) {
                    out.push_str(&crate::app::ui::magenta(&word));
                } else if LITERALS.contains(&word.as_str()) {
                    out.push_str(&crate::app::ui::yellow(&word));
                } else {
                    out.push_str(&word);
                }
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }

    out
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic() || !ch.is_ascii()
}

fn is_ident(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric() || !ch.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ui styling is disabled when stdout is not a TTY, so the highlighted output
    // is byte-for-byte identical to the input in tests.
    #[test]
    fn passes_text_through_unchanged_without_styling() {
        let line = "$sdl->SDL_Init(SDL_INIT_VIDEO); // start";
        assert_eq!(php_line(line), line);
    }

    #[test]
    fn handles_strings_and_comments_without_panicking() {
        assert_eq!(php_line("$x = 'a\\'b';"), "$x = 'a\\'b';");
        assert_eq!(
            php_line("/* note */ return 0x1F;"),
            "/* note */ return 0x1F;"
        );
        assert_eq!(php_line("use Pnlx\\Runtime;"), "use Pnlx\\Runtime;");
    }
}
