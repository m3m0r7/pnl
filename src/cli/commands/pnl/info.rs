//! `pnl info <package>` — describe a package using its *remote* manifest.
//!
//! Unlike `pnl list`, info never inspects the locally installed copy: it always
//! resolves the package from the configured repositories (or the source URL/path
//! given) and reads a fresh `pnlx.json`, so it shows what installing the package
//! *would* do — the native dependency commands it runs, the headers it binds,
//! and the native libraries it links — even when the package is already
//! installed.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::git_source::{GitSource, install_git_source};
use crate::io::{read_json, read_or_default};
use crate::manifest::{NativeRequirement, PnlManifest, PnlxManifest};

use super::install::{is_bare_package_name, resolved_repositories};

pub(super) fn info(root: &Path, target: &str) -> Result<()> {
    let (manifest, source) = resolve_remote_manifest(root, target)?;
    print_info(&manifest, &source);
    Ok(())
}

/// Fetch a package's manifest from its remote source. Bare names are resolved
/// against the configured repositories (highest priority first); everything else
/// is treated as a git source and cloned shallowly into a temporary directory.
fn resolve_remote_manifest(root: &Path, target: &str) -> Result<(PnlxManifest, String)> {
    if is_bare_package_name(target) {
        let manifest = read_or_default::<PnlManifest>(&root.join("pnl.json"))?;
        let mut failures = Vec::new();
        for repository in resolved_repositories(&manifest) {
            let candidate = format!("{}/{target}", repository.url.trim_end_matches('/'));
            crate::ui::debug(&format!("trying {candidate}"));
            match fetch_git_manifest(&candidate) {
                Ok(manifest) => return Ok((manifest, candidate)),
                Err(error) => failures.push(format!("  - {}: {error}", repository.url)),
            }
        }
        bail!(
            "could not find package \"{target}\" in any configured repository:\n{}",
            failures.join("\n")
        );
    }

    crate::ui::debug(&format!("fetching manifest from {target}"));
    Ok((fetch_git_manifest(target)?, target.to_owned()))
}

/// Clone `source_url` shallowly and read the `pnlx.json` at its package path. The
/// temporary checkout is removed when the returned manifest goes out of scope.
fn fetch_git_manifest(source_url: &str) -> Result<PnlxManifest> {
    let source = GitSource::parse(source_url)?;
    let installed = install_git_source(&source)?;
    let manifest_path = installed
        .destination
        .join(&source.package_path)
        .join("pnlx.json");
    if !manifest_path.is_file() {
        bail!("{source_url} does not contain pnlx.json at the requested package path");
    }
    read_json::<PnlxManifest>(&manifest_path)
        .with_context(|| format!("failed to read the pnlx.json of {source_url}"))
    // `installed` drops here, removing the temporary clone.
}

fn print_info(manifest: &PnlxManifest, source: &str) {
    crate::ui::heading("pnl", &format!("info {}", manifest.name));

    field("Name", &manifest.name);
    field("Version", &manifest.version);
    if !manifest.description.is_empty() {
        field("Description", &manifest.description);
    }
    if !manifest.license.is_empty() {
        field("License", &manifest.license);
    }
    if !manifest.authors.is_empty() {
        let authors = manifest
            .authors
            .iter()
            .map(|author| author.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        field("Authors", &authors);
    }
    field("Source", &crate::ui::dim(source).to_string());

    print_install_steps();
    print_native_dependencies(manifest);
    print_requirements(manifest);
}

/// A high-level outline of what `pnl install <package>` performs.
fn print_install_steps() {
    section("What `pnl install` does");
    crate::ui::step("install the native libraries/headers (commands below, if any)");
    crate::ui::step("resolve each native library and header from your library paths");
    crate::ui::step("generate PHP FFI bindings and a Rust bridge for the headers");
    crate::ui::step("compile the bridge and record everything in the lockfile/pathmap");
}

/// The per-platform native-dependency install recipes.
fn print_native_dependencies(manifest: &PnlxManifest) {
    if manifest.installation.is_empty() {
        return;
    }
    section("Native dependency commands");
    for (platform, entry) in &manifest.installation {
        println!("  {}", crate::ui::bold(platform));
        for command in &entry.install {
            println!("    {} {command}", crate::ui::cyan("$"));
        }
        for check in &entry.check_if_exists {
            println!(
                "    {} {}",
                crate::ui::dim("already-present check:"),
                crate::ui::dim(check)
            );
        }
    }
}

/// The native libraries and headers each `requires` entry links/binds.
fn print_requirements(manifest: &PnlxManifest) {
    if manifest.requires.is_empty() {
        return;
    }
    section("Native libraries referenced");
    for (key, requirement) in &manifest.requires {
        println!(
            "  {} {}",
            crate::ui::bold(key),
            crate::ui::dim(&format!("(version {})", requirement.version))
        );
        let libraries = requirement
            .library_names
            .iter()
            .map(|name| name.name().to_owned())
            .collect::<Vec<_>>()
            .join(", ");
        sub_field("library", &libraries);
        print_headers(requirement);
        if let Some(prefix) = &requirement.symbol_prefix {
            sub_field("symbols", &format!("{prefix}*"));
        }
        if let Some(url) = &requirement.library_url {
            sub_field("library url", url);
        }
        if let Some(url) = &requirement.header_url {
            sub_field("header url", url);
        }
    }
}

fn print_headers(requirement: &NativeRequirement) {
    if !requirement.header_names.is_empty() {
        sub_field("headers", &requirement.header_names.join(", "));
    } else if requirement.header_inline.is_some() {
        sub_field("headers", "(inline header embedded in the manifest)");
    }
}

fn field(label: &str, value: &str) {
    println!("  {} {value}", crate::ui::cyan(&format!("{label:<12}")));
}

fn sub_field(label: &str, value: &str) {
    println!("    {} {value}", crate::ui::dim(&format!("{label}:")));
}

fn section(title: &str) {
    println!("\n  {}", crate::ui::bold(title));
}
