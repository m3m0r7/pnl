//! `pnl -i` / `pnlx -i` (information) and `pnl -l` / `pnlx -l` (license).
//!
//! Information prints a neofetch-style banner: ASCII-art logo on the left,
//! environment plus installed-extension details on the right. License prints
//! the LICENSE file (embedded at build time) followed by the third-party
//! components that require attribution.

use std::path::Path;

use anyhow::Result;

use crate::app::ui;
use crate::model::manifest::PnlLock;
use crate::model::workspace::workspace_dir;
use crate::util::io::read_json;

/// The repository LICENSE, copied into the binary at build time.
const LICENSE_TEXT: &str = include_str!("../../../LICENSE");

const REPOSITORY_URL: &str = "https://github.com/m3m0r7/pnl";
const PACKAGES_URL: &str = "https://github.com/m3m0r7/pnl-packages";

/// Direct runtime crate dependencies. Keep in sync with `[dependencies]` in
/// Cargo.toml (dev-dependencies are not distributed and are not listed).
const RUST_LICENSES: &[(&str, &str)] = &[
    ("anyhow", "MIT OR Apache-2.0"),
    ("chrono", "MIT OR Apache-2.0"),
    ("clang", "Apache-2.0"),
    ("clap", "MIT OR Apache-2.0"),
    ("flate2", "MIT OR Apache-2.0"),
    ("gethostname", "Apache-2.0"),
    ("git-url-parse", "MIT"),
    ("git2", "MIT OR Apache-2.0"),
    ("handlebars", "MIT"),
    ("include_dir", "MIT"),
    ("jsonschema", "MIT"),
    ("object", "Apache-2.0 OR MIT"),
    ("semver", "MIT OR Apache-2.0"),
    ("serde", "MIT OR Apache-2.0"),
    ("serde_json", "MIT OR Apache-2.0"),
    ("sha2", "MIT OR Apache-2.0"),
    ("suppaftp", "MIT OR Apache-2.0"),
    ("tar", "MIT OR Apache-2.0"),
    ("ureq", "MIT OR Apache-2.0"),
    ("url", "MIT OR Apache-2.0"),
    ("yaml-rust2", "MIT OR Apache-2.0"),
    ("zip", "MIT"),
];

/// Native libraries vendored into the binary or loaded at runtime.
const NATIVE_LICENSES: &[(&str, &str)] = &[
    ("libgit2 (vendored)", "GPL-2.0 with GCC Linking Exception"),
    ("OpenSSL (vendored on Unix)", "Apache-2.0"),
    (
        "libclang (loaded at runtime, not bundled)",
        "Apache-2.0 WITH LLVM-exception",
    ),
];

/// Third-party PHP runtime dependencies of the SDK. The SDK is self-contained
/// (it relies only on the PHP runtime and the bundled native binaries), so there
/// are none; this stays here as the place to list any that are added later.
const PHP_LICENSES: &[(&str, &str)] = &[];

#[derive(Debug, Clone, Copy)]
pub enum Tool {
    Pnl,
    Pnlx,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Self::Pnl => "pnl",
            Self::Pnlx => "pnlx",
        }
    }

    fn art(self) -> &'static [&'static str] {
        match self {
            Self::Pnl => &[
                "██████╗ ███╗   ██╗██╗",
                "██╔══██╗████╗  ██║██║",
                "██████╔╝██╔██╗ ██║██║",
                "██╔═══╝ ██║╚██╗██║██║",
                "██║     ██║ ╚████║███████╗",
                "╚═╝     ╚═╝  ╚═══╝╚══════╝",
            ],
            Self::Pnlx => &[
                "██████╗ ███╗   ██╗██╗     ██╗  ██╗",
                "██╔══██╗████╗  ██║██║     ╚██╗██╔╝",
                "██████╔╝██╔██╗ ██║██║      ╚███╔╝ ",
                "██╔═══╝ ██║╚██╗██║██║      ██╔██╗ ",
                "██║     ██║ ╚████║███████╗██╔╝ ██╗",
                "╚═╝     ╚═╝  ╚═══╝╚══════╝╚═╝  ╚═╝",
            ],
        }
    }
}

pub fn print_information(tool: Tool) -> Result<()> {
    let art = tool.art();
    let art_width = art
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let info = information_lines(tool);

    println!();
    for row in 0..art.len().max(info.len()) {
        let art_line = art.get(row).copied().unwrap_or("");
        let padding = " ".repeat(art_width - art_line.chars().count());
        println!(
            "  {}{padding}   {}",
            ui::magenta(art_line),
            info.get(row).cloned().unwrap_or_default()
        );
    }
    println!();
    Ok(())
}

/// The right-hand column of the information banner.
fn information_lines(tool: Tool) -> Vec<String> {
    let title = format!("{} {}", tool.name(), env!("CARGO_PKG_VERSION"));
    let mut lines = vec![
        ui::bold(&title),
        ui::dim(&"─".repeat(title.chars().count())),
        entry(
            "OS",
            &format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH),
        ),
        entry("Host", &gethostname::gethostname().to_string_lossy()),
        entry("Binary", &binary_location()),
        entry("Repository", REPOSITORY_URL),
        entry("Packages", PACKAGES_URL),
        entry(
            "License",
            &format!("MIT (run `{} --license` for details)", tool.name()),
        ),
        entry("Copyright", copyright_line(LICENSE_TEXT)),
        entry("Toolchain", &toolchain_status()),
    ];

    let extensions = installed_extensions(Path::new("."));
    if extensions.is_empty() {
        lines.push(entry("Extensions", "(none installed in this workspace)"));
    } else {
        for (index, extension) in extensions.iter().enumerate() {
            let label = if index == 0 { "Extensions" } else { "" };
            lines.push(entry(label, extension));
        }
    }
    lines
}

/// Readiness of the only external requirement for `pnl install` (libclang), plus
/// whether a C compiler — used opportunistically for fuller library discovery — is
/// present. `.pc` parsing is built in, so pkg-config is intentionally not listed.
fn toolchain_status() -> String {
    let libclang = if crate::native::header_adapter::libclang_available() {
        ui::green("libclang ✓")
    } else if cfg!(target_os = "macos") {
        ui::yellow("libclang ✗ (run `xcode-select --install`)")
    } else {
        ui::yellow("libclang ✗ (install your clang/llvm dev package)")
    };
    let compiler = match c_compiler() {
        Some(cc) => ui::green(&format!("{cc} ✓")),
        None => ui::dim("C compiler – (optional)"),
    };
    format!("{libclang}, {compiler}")
}

/// The first available C compiler on `PATH`, if any (optional — only sharpens
/// multiarch library-path discovery).
fn c_compiler() -> Option<&'static str> {
    for cc in ["cc", "clang", "gcc"] {
        let ran = std::process::Command::new(cc)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if ran {
            return Some(cc);
        }
    }
    None
}

fn entry(label: &str, value: &str) -> String {
    // Pad before styling: ANSI escape codes must not count toward the width.
    let padded = if label.is_empty() {
        " ".repeat(12)
    } else {
        format!("{:<12}", format!("{label}:"))
    };
    format!("{}{value}", ui::cyan(&padded))
}

/// `Copyright (c) …` from the embedded LICENSE.
fn copyright_line(license: &str) -> &str {
    license
        .lines()
        .find(|line| line.trim_start().starts_with("Copyright"))
        .map(str::trim)
        .unwrap_or("see LICENSE")
}

fn binary_location() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_owned())
}

/// `name version (installed path)` for every extension in the workspace lock.
/// Errors (no lockfile, invalid JSON) degrade to an empty list because the
/// banner must work outside a pnl workspace too.
fn installed_extensions(root: &Path) -> Vec<String> {
    let Ok(lock) = read_json::<PnlLock>(&root.join(crate::model::config::LOCK_FILE)) else {
        return Vec::new();
    };
    let packages = workspace_dir(root).join("packages");
    lock.extensions
        .iter()
        .map(|(name, extension)| {
            let location = packages.join(name).join(&extension.version);
            format!(
                "{name} {} {}",
                extension.version,
                ui::dim(&format!("({})", location.display()))
            )
        })
        .collect()
}

pub fn print_license() -> Result<()> {
    println!("{}", LICENSE_TEXT.trim_end());
    println!();
    println!("{}", ui::bold("Third-party components"));
    println!("{}", "=".repeat(22));

    println!();
    println!("Rust crates (runtime dependencies):");
    print_license_table(RUST_LICENSES);

    println!();
    println!("Bundled or dynamically loaded native libraries:");
    print_license_table(NATIVE_LICENSES);

    println!();
    println!("PHP packages (runtime dependencies of the PHP SDK):");
    print_license_table(PHP_LICENSES);

    println!();
    println!(
        "Full third-party license texts are distributed with each component's source; see {REPOSITORY_URL}."
    );
    Ok(())
}

fn print_license_table(table: &[(&str, &str)]) {
    let width = table
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    for (name, license) in table {
        println!("  {name:<width$}  {license}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_copyright_line_from_the_license() {
        assert!(copyright_line(LICENSE_TEXT).starts_with("Copyright (c)"));
        assert_eq!(copyright_line("no such line"), "see LICENSE");
    }

    #[test]
    fn entries_align_values_to_a_fixed_column() {
        // ui styling is disabled when stdout is not a TTY, so the padded
        // label is returned verbatim here.
        assert_eq!(entry("OS", "macos"), "OS:         macos");
        assert_eq!(
            entry("", "continued"),
            format!("{}continued", " ".repeat(12))
        );
    }
}
