//! Small terminal-output helpers for a friendly, npm-like CLI.
//!
//! Styling (ANSI colours and symbols) is applied only when stdout is a TTY and
//! `NO_COLOR` is unset, so piped/CI output stays plain and stable.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::OnceLock;

fn styled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
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

fn green(text: &str) -> String {
    sgr("32", text)
}

pub fn cyan(text: &str) -> String {
    sgr("36", text)
}

fn yellow(text: &str) -> String {
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

/// Print a labeled usage-example block (the example body is shown verbatim).
pub fn example_block(label: &str, body: &str) {
    println!("\n  {} {}", dim("›"), bold(label));
    for line in body.lines() {
        println!("    {line}");
    }
}

/// Format a `Duration` like npm's `in 1.234s`.
pub fn elapsed(duration: std::time::Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}
