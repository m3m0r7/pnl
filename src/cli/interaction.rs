use std::io::{self, IsTerminal, Write};

use anyhow::Result;

/// Carries whether the user asked to skip interactive prompts (`--no-interaction`).
///
/// Prompts also fall back to their default when stdin is not a TTY, so piped or
/// CI invocations never block waiting for input.
#[derive(Debug, Clone, Copy, Default)]
pub struct Interaction {
    no_interaction: bool,
}

impl Interaction {
    pub fn new(no_interaction: bool) -> Self {
        Self { no_interaction }
    }

    /// Ask a yes/no question. Returns `default` without prompting when
    /// `--no-interaction` was given or stdin is not interactive; an empty answer
    /// also selects the default.
    pub fn confirm(&self, question: &str, default: bool) -> Result<bool> {
        if self.no_interaction || !io::stdin().is_terminal() {
            return Ok(default);
        }

        let hint = if default { "[Y/n]" } else { "[y/N]" };
        loop {
            print!("{question} {hint} ");
            io::stdout().flush()?;

            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 {
                // EOF: behave like a non-interactive run.
                return Ok(default);
            }

            match answer.trim().to_ascii_lowercase().as_str() {
                "" => return Ok(default),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => println!("Please answer 'y' or 'n'."),
            }
        }
    }
}
