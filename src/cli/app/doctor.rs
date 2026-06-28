//! `pnl doctor` — diagnose the local environment for installing and running pnl
//! extensions: the binding-generation toolchain (libclang), the optional C
//! compiler, the PHP runtime and its FFI extension, and the current workspace.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::app::ui;
use crate::model::manifest::PnlLock;
use crate::util::io::read_json;

enum Status {
    Ok,
    Warn,
    Fail,
}

fn line(status: &Status, label: &str, detail: &str) {
    let symbol = match status {
        Status::Ok => ui::green("✓"),
        Status::Warn => ui::yellow("⚠"),
        Status::Fail => ui::red("✗"),
    };
    println!("  {symbol} {} {detail}", ui::cyan(&format!("{label:<13}")));
}

/// Stdout of `program args`, trimmed, or `None` if it cannot run or exits non-zero.
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The first available C compiler on `PATH` (used only for `static_inline` shims).
fn c_compiler() -> Option<&'static str> {
    ["cc", "clang", "gcc"]
        .into_iter()
        .find(|cc| command_output(cc, &["--version"]).is_some())
}

pub fn run(root: &Path) -> Result<()> {
    ui::heading("pnl", "doctor");
    println!();
    let mut failures = 0u32;

    // libclang — the only hard requirement for `pnl install` (binding generation).
    match crate::native::header_adapter::ensure_libclang_available() {
        Ok(()) => line(
            &Status::Ok,
            "libclang",
            "available (required to generate bindings)",
        ),
        Err(error) => {
            failures += 1;
            line(
                &Status::Fail,
                "libclang",
                &ui::red("not found — `pnl install` cannot generate bindings"),
            );
            for detail in error.to_string().lines() {
                println!("      {}", ui::dim(detail));
            }
        }
    }

    // C compiler — optional, only for `compile_options.static_inline` shims.
    match c_compiler() {
        Some(cc) => line(
            &Status::Ok,
            "C compiler",
            &format!("{cc} (optional — for compile_options.static_inline)"),
        ),
        None => line(
            &Status::Warn,
            "C compiler",
            "none on PATH (optional — only needed for compile_options.static_inline)",
        ),
    }

    // pkg-config is NOT required: pnl parses `.pc` files itself.
    line(
        &Status::Ok,
        "pkg-config",
        &ui::dim("built-in (.pc parsed directly; the system pkg-config is not required)"),
    );

    // PHP runtime + the FFI extension the generated SDK loads libraries through.
    match command_output("php", &["-r", "echo PHP_VERSION;"]) {
        Some(version) => {
            line(&Status::Ok, "PHP", &version);
            let ffi_loaded =
                command_output("php", &["-r", "echo extension_loaded('ffi') ? '1' : '0';"])
                    .as_deref()
                    == Some("1");
            if ffi_loaded {
                let enable = command_output("php", &["-r", "echo ini_get('ffi.enable');"])
                    .unwrap_or_default();
                let note = match enable.as_str() {
                    "" | "preload" => {
                        "ffi.enable=preload (FFI works in CLI; set ffi.enable=1 for web SAPIs)"
                            .to_owned()
                    }
                    "1" | "true" => "ffi.enable=1".to_owned(),
                    other => format!("ffi.enable={other}"),
                };
                line(&Status::Ok, "PHP FFI", &note);
            } else {
                failures += 1;
                line(
                    &Status::Fail,
                    "PHP FFI",
                    &ui::red("ext-ffi not loaded — enable the FFI extension in php.ini"),
                );
            }
        }
        None => {
            failures += 1;
            line(
                &Status::Fail,
                "PHP",
                &ui::red("not found on PATH — install PHP 8.1+ with ext-ffi"),
            );
        }
    }

    // Workspace — informational: present config and how many extensions are locked.
    let manifest_path = root.join(crate::model::config::PNL_MANIFEST_FILE);
    if manifest_path.is_file() {
        let lock_path = root.join(crate::model::config::LOCK_FILE);
        let locked = read_json::<PnlLock>(&lock_path)
            .map(|lock| lock.extensions.len())
            .unwrap_or(0);
        line(
            &Status::Ok,
            "workspace",
            &format!("pnl.json found ({locked} extension(s) locked)"),
        );
    } else {
        line(
            &Status::Warn,
            "workspace",
            &ui::dim("no pnl.json in this directory (run `pnl init` to create one)"),
        );
    }

    println!();
    if failures == 0 {
        ui::success("all required checks passed");
        Ok(())
    } else {
        anyhow::bail!("{failures} required check(s) failed");
    }
}
