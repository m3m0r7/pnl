//! Small terminal-output helpers for a friendly, npm-like CLI.
//!
//! Styling (ANSI colours and symbols) is applied only when stdout is a TTY and
//! `NO_COLOR` is unset, so piped/CI output stays plain and stable.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

fn styled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

/// Whether `--debug` was requested; gates [`debug`] output.
static DEBUG: AtomicBool = AtomicBool::new(false);

/// Enable or disable verbose `--debug` diagnostics for the rest of the run.
pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

/// True when `--debug` is active.
pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

/// A muted `debug:` diagnostic line (to stderr), shown only under `--debug`.
pub fn debug(message: &str) {
    if debug_enabled() {
        eprintln!("  {} {}", dim("debug"), dim(message));
    }
}

fn sgr(code: &str, text: &str) -> String {
    if styled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

pub fn dim(text: &str) -> String {
    sgr("2", text)
}

pub fn bold(text: &str) -> String {
    sgr("1", text)
}

pub fn green(text: &str) -> String {
    sgr("32", text)
}

pub fn red(text: &str) -> String {
    sgr("31", text)
}

pub fn cyan(text: &str) -> String {
    sgr("36", text)
}

pub fn yellow(text: &str) -> String {
    sgr("33", text)
}

pub fn magenta(text: &str) -> String {
    sgr("35", text)
}

/// A command banner, e.g. `pnl install`.
pub fn heading(tool: &str, action: &str) {
    println!("\n{} {}", magenta(&bold(tool)), bold(action));
}

/// A ✓ line for a completed step.
pub fn success(message: &str) {
    println!("  {} {message}", green("✓"));
}

/// A ✓ line for a generated/written file: `✓ generated <path>`.
pub fn created(label: &str, path: &Path) {
    println!(
        "  {} {label} {}",
        green("✓"),
        dim(&path.display().to_string())
    );
}

/// A › line for an in-progress step.
pub fn step(message: &str) {
    println!("  {} {message}", cyan("›"));
}

/// A muted informational line.
pub fn info(message: &str) {
    println!("  {} {}", dim("•"), dim(message));
}

/// A ⚠ warning line (to stderr).
pub fn warn(message: &str) {
    eprintln!("  {} {message}", yellow("⚠"));
}

/// A bold closing summary, e.g. `added 1 extension in 1.20s`.
pub fn summary(message: &str) {
    println!("\n{}", bold(message));
}

/// Print a labeled usage-example block. The body is treated as lightweight
/// Markdown: fenced ```php blocks are syntax-highlighted, other fenced blocks
/// are dimmed, `#` headings are bolded, and the fence markers themselves are
/// drawn as a subtle rule so the code stands out.
pub fn example_block(label: &str, body: &str) {
    println!("\n  {} {}", dim("›"), bold(label));

    // The active fenced-code language, if we are inside a ``` block.
    let mut fence: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if fence.is_some() {
                fence = None;
                println!("    {}", dim("╰┄┄┄"));
            } else {
                let lang = rest.trim().to_owned();
                let header = if lang.is_empty() {
                    "╭┄┄┄".to_owned()
                } else {
                    format!("╭┄┄┄ {lang}")
                };
                println!("    {}", dim(&header));
                fence = Some(lang);
            }
            continue;
        }

        match fence.as_deref() {
            Some("php") => println!("    {} {}", dim("┊"), crate::app::highlight::php_line(line)),
            Some(_) => println!("    {} {}", dim("┊"), dim(line)),
            None if trimmed.starts_with('#') => {
                println!("    {}", bold(trimmed.trim_start_matches('#').trim_start()))
            }
            None => println!("    {line}"),
        }
    }
}

/// Draw a rounded box around `lines` (to stderr), with an optional bold `title`
/// row. The box width follows the widest visible line (ANSI escapes are not
/// counted). It writes to stderr so it never pollutes piped stdout.
pub fn notice_box(title: &str, lines: &[String]) {
    let mut rows: Vec<String> = Vec::new();
    if !title.is_empty() {
        rows.push(bold(title));
    }
    rows.extend(lines.iter().cloned());

    let width = rows.iter().map(|row| visible_width(row)).max().unwrap_or(0);
    eprintln!("\n{}", yellow(&format!("╭{}╮", "─".repeat(width + 2))));
    for row in &rows {
        let padding = " ".repeat(width.saturating_sub(visible_width(row)));
        eprintln!("{} {row}{padding} {}", yellow("│"), yellow("│"));
    }
    eprintln!("{}", yellow(&format!("╰{}╯", "─".repeat(width + 2))));
}

/// The printable width of a string, ignoring ANSI SGR escape sequences and
/// approximating the double-width cells most terminals give emoji.
fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip until the terminating 'm' of a CSI SGR sequence.
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            width += char_width(ch);
        }
    }
    width
}

/// A rough terminal cell width for a character: 0 for the emoji variation
/// selector, 2 for emoji/dingbat symbols (which render double-wide), 1
/// otherwise. Box-drawing characters (U+2500–U+25FF) stay single-width.
fn char_width(ch: char) -> usize {
    match ch as u32 {
        0xFE0F => 0,
        0x2600..=0x27BF | 0x1F000..=0x1FAFF => 2,
        _ => 1,
    }
}

/// Format a `Duration` like npm's `in 1.234s`.
pub fn elapsed(duration: std::time::Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_width_ignores_ansi_and_counts_emoji_as_two() {
        // styling is disabled outside a TTY, so build escapes explicitly here.
        assert_eq!(visible_width("\x1b[1mpnl\x1b[0m"), 3);
        assert_eq!(visible_width("✨ ok"), 5); // 2 + space + 2 letters
        assert_eq!(visible_width("📦 1.0"), 6); // 2 + space + 3 chars
        // Box-drawing characters stay single width.
        assert_eq!(visible_width("│──│"), 4);
    }

    #[test]
    fn renderers_do_not_panic() {
        // Smoke coverage: exercising the formatting paths must not panic, even on
        // unterminated fences and empty content.
        example_block("usage", "intro\n```php\n$x = 1; // c\n```\nplain\n```\nraw");
        example_block("dangling", "```php\n$open = true;");
        notice_box("✨ title", &[String::new(), "📦 line".to_owned()]);
    }
}
