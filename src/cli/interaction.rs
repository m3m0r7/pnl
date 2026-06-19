use std::io::{self, IsTerminal};

use anyhow::Result;

/// Carries whether the user asked to skip interactive prompts (`--no-interaction`).
///
/// Prompts also fall back to their default when stdin is not a TTY, so piped or
/// CI invocations never block waiting for input.
#[derive(Debug, Clone, Copy, Default)]
pub struct Interaction {
    no_interaction: bool,
    assume_yes: bool,
}

impl Interaction {
    pub fn new(no_interaction: bool, assume_yes: bool) -> Self {
        Self {
            no_interaction,
            assume_yes,
        }
    }

    pub fn assume_yes(&self) -> bool {
        self.assume_yes
    }

    /// Ask a yes/no question. Returns `true` without prompting when `--yes` was
    /// given; returns `default` without prompting when `--no-interaction` was
    /// given or stdin is not interactive; an empty answer also selects the default.
    pub fn confirm(&self, question: &str, default: bool) -> Result<bool> {
        if self.assume_yes {
            return Ok(true);
        }
        if self.no_interaction || !io::stdin().is_terminal() {
            return Ok(default);
        }
        select_yes_no(question, default)
    }

    /// Whether prompts can actually read interactive input: stdin is a TTY and the
    /// caller asked for neither `--no-interaction` nor `--yes`. Callers that need a
    /// typed value (not just yes/no) check this and fall back to a default otherwise,
    /// so piped/CI installs never block.
    pub fn can_prompt(&self) -> bool {
        !self.no_interaction && !self.assume_yes && io::stdin().is_terminal()
    }

    /// Prompt for a free-text value on stderr, showing `description` and the current
    /// `initial` value (used on an empty answer). Returns the trimmed input. Only
    /// call when [`can_prompt`](Self::can_prompt) is true.
    pub fn read_value(
        &self,
        label: &str,
        description: &str,
        initial: Option<&str>,
    ) -> Result<String> {
        use console::style;
        use std::io::Write;

        let mut err = io::stderr();
        writeln!(err, "{} {label}", style("?").cyan().bold())?;
        if !description.is_empty() {
            writeln!(err, "{}", style(format!("  {description}")).dim())?;
        }
        match initial {
            Some(value) => write!(err, "  {} ", style(format!("[{value}]:")).dim())?,
            None => write!(err, "  > ")?,
        }
        err.flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(line.trim().to_owned())
    }
}

/// An arrow-key Yes/No selector rendered on stderr (↑/↓ or y/n, Enter to
/// confirm). Falls back to its default when the terminal can't provide key
/// events.
fn select_yes_no(question: &str, default: bool) -> Result<bool> {
    use console::{Key, Term, style};

    let term = Term::stderr();
    // Index 0 = Yes, 1 = No; start on the default.
    let mut selected = usize::from(!default);

    term.write_line(&format!("{} {question}", style("?").cyan().bold()))?;
    let mut rendered = false;
    let answer = loop {
        if rendered {
            term.clear_last_lines(3)?;
        }
        for (index, label) in ["Yes", "No"].into_iter().enumerate() {
            if index == selected {
                term.write_line(&format!(
                    "  {} {}",
                    style("›").cyan(),
                    style(label).cyan().bold()
                ))?;
            } else {
                term.write_line(&format!("    {}", style(label).dim()))?;
            }
        }
        term.write_line(&style("  (↑/↓ to move, Enter to confirm)").dim().to_string())?;
        rendered = true;

        match term.read_key() {
            Ok(Key::ArrowUp | Key::ArrowDown | Key::Char('k') | Key::Char('j') | Key::Tab) => {
                selected ^= 1;
            }
            Ok(Key::Char('y' | 'Y')) => break true,
            Ok(Key::Char('n' | 'N')) => break false,
            Ok(Key::Enter) => break selected == 0,
            // Ctrl-C / unreadable input: keep the default rather than blocking.
            Err(_) => break default,
            _ => {}
        }
    };

    // Collapse the selector to a single confirmed line.
    term.clear_last_lines(3)?;
    let shown = if answer { "Yes" } else { "No" };
    term.write_line(&format!(
        "{} {question} {}",
        style("?").green().bold(),
        style(shown).green()
    ))?;
    Ok(answer)
}
