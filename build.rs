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

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use handlebars::Handlebars;
use serde_json::json;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set for build scripts"));
    generate_config_constants(&out_dir);

    // The PHP SDK under `src/sdk` is embedded with `include_dir!` (see
    // `sdk_assets.rs`). On stable Rust that macro does NOT register the embedded
    // files as rebuild triggers, so editing an SDK file alone would otherwise leave
    // a stale runtime baked into the binary (a fixed `Util`/`NativeLibrary` method
    // would silently not ship). Two-part guard: (1) re-export every SDK file as a
    // build-script trigger so this script reruns on any change, and (2) write a
    // content fingerprint that `sdk_assets.rs` pulls in via `include_str!` (which
    // cargo DOES track), so a changed fingerprint forces that module to recompile
    // and `include_dir!` to re-embed the current tree.
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"));
    let mut fingerprint = String::new();
    fingerprint_sdk_tree(&manifest_dir.join("src/sdk"), &mut fingerprint);
    fs::write(out_dir.join("sdk_fingerprint.txt"), fingerprint)
        .expect("failed to write SDK fingerprint");

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
    let authorized_repositories = string_array(
        &["repositories", "authorized_repositories"],
        "repositories.authorized_repositories",
    );

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
        "authorized_repositories": authorized_repositories,
        "authorized_repositories_len": authorized_repositories.len(),
        "pnl_manifest": string(&["filenames", "pnl_manifest"]),
        "pnlx_manifest": string(&["filenames", "pnlx_manifest"]),
        "lockfile": string(&["filenames", "lockfile"]),
        "pathmap": string(&["filenames", "pathmap"]),
        "autoload": string(&["filenames", "autoload"]),
        "generated_dir": string(&["filenames", "generated_dir"]),
        "aliases_file": string(&["filenames", "aliases_file"]),
        "ffi_suffix": string(&["filenames", "ffi_suffix"]),
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

/// Walk `src/sdk` in sorted order, emitting a `rerun-if-changed` for every file
/// and appending each file's path and length to `fingerprint`. The fingerprint is
/// written to OUT_DIR and `include_str!`d by `sdk_assets.rs`, so any SDK edit
/// changes a cargo-tracked input and forces the embedded copy to be rebuilt.
fn fingerprint_sdk_tree(dir: &Path, fingerprint: &mut String) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            fingerprint_sdk_tree(&path, fingerprint);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            let bytes = fs::read(&path).unwrap_or_default();
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            fingerprint.push_str(&format!("{}:{:x}\n", path.display(), hasher.finish()));
        }
    }
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
