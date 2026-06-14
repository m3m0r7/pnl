//! Embeds the compiled support `cdylib` (this crate's own `cdylib` output) into
//! the `pnl`/`pnlx` binaries so `pnl install` can expand it into `@pnlx/runtime`
//! for the PHP runtime to load via FFI.
//!
//! The cdylib and the binaries are produced by the same `cargo build`, so on a
//! single build the cdylib does not yet exist when this script runs — it embeds
//! an empty blob. Release builds (`make build`) run `cargo build` twice: the
//! second pass embeds the cdylib produced by the first. A development build that
//! runs once simply ships an empty blob; `pnl install` then falls back to copying
//! the cdylib sitting next to the executable.
//!
//! It also bakes `config.toml` into compile-time constants (see
//! [`generate_config_constants`]) so the built-in endpoints and defaults travel
//! with the binary without any runtime TOML parsing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use handlebars::Handlebars;
use serde_json::json;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set for build scripts"));
    generate_config_constants(&out_dir);

    let dest = out_dir.join("support.lib");

    let lib_name = match env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "windows" => "pnl.dll",
        "macos" => "libpnl.dylib",
        _ => "libpnl.so",
    };

    // OUT_DIR is `<target>/<profile>/build/<pkg>-<hash>/out`; the cdylib from a
    // prior build sits at `<target>/<profile>/<lib_name>`.
    let mut bytes = Vec::new();
    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        let candidate = profile_dir.join(lib_name);
        if candidate.is_file() {
            println!("cargo:rerun-if-changed={}", candidate.display());
            bytes = fs::read(&candidate).unwrap_or_default();
        }
    }

    fs::write(&dest, &bytes).expect("failed to write embedded support library");
}

/// Parse `config.toml` and render `config_constants.rs` (included by
/// `src/cli/config.rs`) from a Handlebars template, so the built-in defaults are
/// compiled in rather than parsed at runtime — and the Rust we emit is shaped by
/// a template, not assembled with string concatenation.
fn generate_config_constants(out_dir: &Path) {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"));
    let config_path = manifest_dir.join("config.toml");
    let template_path = manifest_dir.join("src/cli/templates/build/config_constants.rs.tpl");
    println!("cargo:rerun-if-changed={}", config_path.display());
    println!("cargo:rerun-if-changed={}", template_path.display());

    let raw = fs::read_to_string(&config_path).expect("failed to read config.toml");
    let config: toml::Table = raw.parse().expect("config.toml is not valid TOML");

    let string = |path: &[&str]| -> String {
        lookup(&config, path)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("config.toml is missing string {}", path.join(".")))
            .to_owned()
    };
    let integer = |path: &[&str]| -> i64 {
        lookup(&config, path)
            .and_then(toml::Value::as_integer)
            .unwrap_or_else(|| panic!("config.toml is missing integer {}", path.join(".")))
    };
    let string_array = |path: &[&str], item_desc: &str| -> Vec<String> {
        lookup(&config, path)
            .and_then(toml::Value::as_array)
            .unwrap_or_else(|| panic!("config.toml is missing array {}", path.join(".")))
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{item_desc} must hold strings"))
                    .to_owned()
            })
            .collect()
    };
    let binaries = string_array(&["binaries", "names"], "binaries.names");
    let authorized = string_array(&["repositories", "authorized"], "repositories.authorized");

    let context = json!({
        "schema_version": string(&["schema_version"]),
        "self_repository": string(&["repositories", "self"]),
        "packages_repository": string(&["repositories", "packages"]),
        "output_dir": string(&["workspace", "output_dir"]),
        "ttl_seconds": integer(&["update_check", "ttl_seconds"]),
        "opt_out_env": string(&["update_check", "opt_out_env"]),
        "cache_key": string(&["update_check", "cache_key"]),
        "binaries": binaries,
        "binaries_len": binaries.len(),
        "authorized": authorized,
        "authorized_len": authorized.len(),
        // The build target, surfaced to the PHP layer as PNLX_BUILD_OS/ARCH.
        "build_os": env::var("CARGO_CFG_TARGET_OS").unwrap_or_default(),
        "build_arch": env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default(),
    });

    let template = fs::read_to_string(&template_path).expect("failed to read config template");
    let mut handlebars = Handlebars::new();
    handlebars.register_escape_fn(handlebars::no_escape);
    let generated = handlebars
        .render_template(&template, &context)
        .expect("failed to render config constants template");

    fs::write(out_dir.join("config_constants.rs"), generated)
        .expect("failed to write generated config constants");
}

/// Walk a dotted path of nested TOML tables, returning the value at the leaf.
fn lookup<'a>(table: &'a toml::Table, path: &[&str]) -> Option<&'a toml::Value> {
    let (first, rest) = path.split_first()?;
    let mut value = table.get(*first)?;
    for key in rest {
        value = value.get(*key)?;
    }
    Some(value)
}
