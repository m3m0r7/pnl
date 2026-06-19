use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

use crate::archive::{extract_extension_archive, is_archive_source};
use crate::commands::pnlx::{generate_installed_package_artifacts, read_existing_ffi_cdef};
use crate::fetch::{fetch_asset, is_remote_source};
use crate::generate::parse_function_signatures;
use crate::git_source::{GitSource, install_git_source};
use crate::io::{read_json, read_or_default, write_json};
use crate::manifest::{
    DefinitionType, Dist, ExtensionRequirement, LockedExtension, LockedNativeLibrary, PnlManifest,
    PnlxManifest, Repository, RepositoryType, RequireDefinition, ResolvedDefinition, Source,
};
use crate::platform::now;
use crate::repository_index::{
    installed_version_satisfies, load_repository_index, select_package_version,
};
use crate::validate::{
    validate_pnl_manifest_values, validate_pnlx_manifest_values, validate_schema_version,
};

use super::native::{
    generation_headers_from_resolved_header, resolve_header_for_native, resolve_native_library,
};
use super::package::{
    absolutize, entity_class_fqn, file_url_for_path, install_extension_files, package_dir_in,
    pnl_lock_path, pnlx_workspace_dir, read_lock_for_current_platform,
    read_pathmap_for_current_platform, tree_sha256, write_pathmap, write_pnlx_autoload,
};

/// Options supplied on the `pnl install` command line.
#[derive(Debug, Clone, Default)]
pub(crate) struct InstallOptions {
    /// Define a `class_alias` for the generated extension class.
    pub alias_class: Option<String>,
    /// Prefix added to every generated function and method name.
    pub function_prefix: Option<String>,
    /// Drives confirmation prompts (e.g. native-dependency installation).
    pub interaction: crate::interaction::Interaction,
    /// Continue when install scripts are missing or fail their publish-time hash
    /// check.
    pub allow_unverified_install_scripts: bool,
    /// Hashes explicitly trusted for this install run.
    pub allowed_install_script_hashes: Vec<String>,
    /// Persist `features.use_functions = true` into pnl.json.
    pub enable_use_functions: bool,
    /// Persist `features.allow_cdata = true` into pnl.json.
    pub enable_allow_cdata: bool,
    /// Persist `features.use_php_scalars_in_params = true` into pnl.json.
    pub enable_use_php_scalars_in_params: bool,
    /// Persist `features.use_php_scalars_in_return = true` into pnl.json.
    pub enable_use_php_scalars_in_return: bool,
    /// Persist `features.use_php_scalars_in_const = true` into pnl.json.
    pub enable_use_php_scalars_in_const: bool,
    /// Reinstall even when the resolved content differs from the lockfile digest,
    /// overwriting the recorded sha256 instead of aborting.
    pub force: bool,
}

#[derive(Debug, Default)]
struct InstallState {
    stack: Vec<String>,
    /// When set, the `packages/` directory the current install writes into — a
    /// parent package's own subtree, for a dependency installed nested under it.
    /// `None` for a top-level install (the workspace `@pnlx/packages`).
    packages_root: Option<std::path::PathBuf>,
}

impl InstallState {
    /// Whether the install in progress is a (nested) dependency rather than a
    /// top-level target, so it is kept out of `pnl.json` and the top-level lock.
    fn is_dependency(&self) -> bool {
        self.packages_root.is_some()
    }
}

/// The `packages/` directory the current install writes into: the parent package's
/// subtree for a dependency, else the workspace `@pnlx/packages`.
fn current_packages_root(root: &Path, state: &InstallState) -> std::path::PathBuf {
    state
        .packages_root
        .clone()
        .unwrap_or_else(|| pnlx_workspace_dir(root).join("packages"))
}

/// Apply `--enable-use-functions` / `--enable-allow-cdata` to the manifest,
/// returning whether anything changed (so the caller persists pnl.json).
fn apply_feature_flags(manifest: &mut PnlManifest, options: &InstallOptions) -> bool {
    let mut changed = false;
    if options.enable_use_functions && !manifest.features.use_functions {
        manifest.features.use_functions = true;
        changed = true;
    }
    if options.enable_allow_cdata && !manifest.features.allow_cdata {
        manifest.features.allow_cdata = true;
        changed = true;
    }
    if options.enable_use_php_scalars_in_params && !manifest.features.use_php_scalars_in_params {
        manifest.features.use_php_scalars_in_params = true;
        changed = true;
    }
    if options.enable_use_php_scalars_in_return && !manifest.features.use_php_scalars_in_return {
        manifest.features.use_php_scalars_in_return = true;
        changed = true;
    }
    if options.enable_use_php_scalars_in_const && !manifest.features.use_php_scalars_in_const {
        manifest.features.use_php_scalars_in_const = true;
        changed = true;
    }
    changed
}

/// Resolve a package's `require_definitions` to concrete values. Resolution order
/// (per the lock-as-source-of-truth model): a value already recorded in the lock
/// (`prior`) preseeds the prompt and is the value a non-interactive install uses;
/// otherwise the declared `default`. An interactive install always prompts
/// (preseeded). A non-interactive install with neither a prior value nor a default
/// errors instead of guessing.
fn resolve_require_definitions(
    package: &str,
    definitions: &[RequireDefinition],
    prior: &BTreeMap<String, String>,
    interaction: &crate::interaction::Interaction,
) -> Result<Vec<ResolvedDefinition>> {
    let mut resolved = Vec::new();
    for definition in definitions {
        let initial = prior
            .get(&definition.name)
            .cloned()
            .or_else(|| definition.default.as_ref().map(definition_default_string));
        let value = if interaction.can_prompt() {
            prompt_definition(definition, initial.as_deref(), interaction)?
        } else {
            let value = initial.clone().ok_or_else(|| {
                anyhow!(
                    "{package} requires the build-time definition `{name}`, but no value \
                     is available (no default, none recorded in pnlx-lock.json, and this \
                     is a non-interactive install). Run install interactively or record a \
                     value in the lockfile.",
                    name = definition.name,
                )
            })?;
            validate_definition_value(definition.definition_type, &value)
                .map_err(|reason| anyhow!("invalid value for `{}`: {reason}", definition.name))?;
            value
        };
        resolved.push(ResolvedDefinition {
            name: definition.name.clone(),
            value,
            definition_type: definition.definition_type,
        });
    }
    Ok(resolved)
}

/// Prompt for one definition's value, re-asking until it validates. A `boolean`
/// uses the Y/n selector; the other types read a typed line (empty keeps the
/// preseeded `initial`).
fn prompt_definition(
    definition: &RequireDefinition,
    initial: Option<&str>,
    interaction: &crate::interaction::Interaction,
) -> Result<String> {
    if definition.definition_type == DefinitionType::Boolean {
        let question = if definition.description.is_empty() {
            definition.name.clone()
        } else {
            format!("{} — {}", definition.name, definition.description)
        };
        let yes = interaction.confirm(&question, matches!(initial, Some("1")))?;
        return Ok(if yes { "1" } else { "0" }.to_owned());
    }
    loop {
        let raw = interaction.read_value(&definition.name, &definition.description, initial)?;
        let candidate = if raw.is_empty() {
            initial.map(str::to_owned)
        } else {
            Some(normalize_definition_value(definition.definition_type, &raw))
        };
        let Some(value) = candidate else {
            crate::ui::warn("a value is required");
            continue;
        };
        match validate_definition_value(definition.definition_type, &value) {
            Ok(()) => return Ok(value),
            Err(reason) => crate::ui::warn(&reason),
        }
    }
}

/// A declared JSON default rendered as the string the solver carries (a boolean as
/// `1`/`0`, a string verbatim, a number as its text).
fn definition_default_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(flag) => if *flag { "1" } else { "0" }.to_owned(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Normalise raw input to the canonical stored form (a boolean to `1`/`0`).
fn normalize_definition_value(definition_type: DefinitionType, raw: &str) -> String {
    if definition_type != DefinitionType::Boolean {
        return raw.trim().to_owned();
    }
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "y" | "yes" | "true" | "t" | "on" => "1".to_owned(),
        "0" | "n" | "no" | "false" | "f" | "off" => "0".to_owned(),
        other => other.to_owned(),
    }
}

/// Validate a solved value against its declared type.
fn validate_definition_value(
    definition_type: DefinitionType,
    value: &str,
) -> std::result::Result<(), String> {
    match definition_type {
        DefinitionType::Int => value
            .parse::<i128>()
            .map(|_| ())
            .map_err(|_| format!("`{value}` is not an integer")),
        DefinitionType::Float => value
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("`{value}` is not a number")),
        DefinitionType::String => Ok(()),
        DefinitionType::Boolean => (value == "0" || value == "1")
            .then_some(())
            .ok_or_else(|| format!("`{value}` is not a boolean (enter y/n)")),
    }
}

pub(super) fn install(root: &Path, targets: &[String], options: &InstallOptions) -> Result<()> {
    let mut manifest =
        read_or_default::<PnlManifest>(&root.join(crate::config::PNL_MANIFEST_FILE))?;
    validate_schema_version(&manifest.schema_version)?;
    validate_pnl_manifest_values(&manifest)?;

    // Persist feature toggles up front so `pnl install --enable-…` works even with
    // no target (the manifest is rewritten again when an extension is added).
    if apply_feature_flags(&mut manifest, options) {
        write_json(&root.join(crate::config::PNL_MANIFEST_FILE), &manifest)?;
    }

    if targets.is_empty() {
        // `pnl install` with no target restores every extension from the lockfile.
        return restore_from_lock(root, &mut manifest, options);
    }

    let label = if targets.len() == 1 {
        format!("install {}", targets[0])
    } else {
        format!("install {} packages", targets.len())
    };
    crate::ui::heading("pnl", &label);
    let started = std::time::Instant::now();
    let mut state = InstallState::default();

    for target in targets {
        // An optional `@<version>` suffix pins the version (e.g. `…/widget@1.2.3`).
        let (target, pinned_version) = split_version_pin(target);
        if targets.len() > 1 {
            crate::ui::step(target);
        }
        // `@<version>` both checks out that git ref and asserts the resolved version.
        install_one(
            root,
            &mut manifest,
            target,
            pinned_version,
            pinned_version,
            options,
            &mut state,
            None,
        )?;
    }

    offer_gitignore(root, &options.interaction)?;

    crate::ui::summary(&format!(
        "added {} extension(s) in {}",
        targets.len(),
        crate::ui::elapsed(started.elapsed())
    ));
    Ok(())
}

/// Offer to add the generated workspace directory (`@pnlx`) to `.gitignore` — it
/// is regenerable and should not be committed. No-op when it is already ignored,
/// the user declines, or the prompt is non-interactive.
pub(super) fn offer_gitignore(
    root: &Path,
    interaction: &crate::interaction::Interaction,
) -> Result<()> {
    let output_dir = crate::workspace::output_dir_name(root);
    let gitignore = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();

    let already_ignored = existing.lines().map(str::trim).any(|line| {
        let line = line.trim_start_matches('/').trim_end_matches('/');
        line == output_dir
    });
    if already_ignored {
        return Ok(());
    }

    if !interaction.confirm(&format!("Add {output_dir}/ to .gitignore?"), true)? {
        return Ok(());
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("/{output_dir}/\n"));
    std::fs::write(&gitignore, content)
        .with_context(|| format!("failed to write {}", gitignore.display()))?;
    crate::ui::created("updated", &gitignore);
    Ok(())
}

/// Resolve a single install target and install it.
///
/// `git_ref` (when set) checks out that branch/tag for git sources. `expected_version`
/// (when set) asserts the resolved package version — these differ for a lockfile
/// restore, where the git ref stays on the source's branch but the version must
/// still match what was locked.
#[allow(clippy::too_many_arguments)]
fn install_one(
    root: &Path,
    manifest: &mut PnlManifest,
    target: &str,
    git_ref: Option<&str>,
    expected_version: Option<&str>,
    options: &InstallOptions,
    state: &mut InstallState,
    expected_content_hash: Option<&str>,
) -> Result<()> {
    // A bare package name (no scheme/slash, not a local package dir) is resolved
    // against the configured repositories, e.g. `pnl install widget`.
    if is_bare_package_name(target)
        && !absolutize(root, Path::new(target))
            .join(crate::config::PNLX_MANIFEST_FILE)
            .is_file()
    {
        return install_bare_name(
            root,
            manifest,
            target,
            version_constraint_for_expected(expected_version).as_deref(),
            git_ref,
            expected_version,
            options,
            state,
        );
    }

    match resolve_install_source(root, target)? {
        InstallSource::Local { path, source_url } => install_local_extension(
            root,
            manifest,
            &path,
            ExtensionSource::File { source_url },
            expected_version,
            options,
            state,
            expected_content_hash,
        ),
        InstallSource::Git(mut source) => {
            // Pin the clone to the requested tag/branch.
            if let Some(reference) = git_ref {
                source.branch = Some(reference.to_owned());
            }
            install_git_extension(
                root,
                manifest,
                target,
                source,
                expected_version,
                options,
                state,
                expected_content_hash,
            )
        }
    }
}

/// Reinstall every extension recorded in the lockfile, pinned to its locked
/// version. The per-extension content digest is verified against the lock, so a
/// source whose content has drifted from the recorded sha256 aborts the install.
fn restore_from_lock(
    root: &Path,
    manifest: &mut PnlManifest,
    options: &InstallOptions,
) -> Result<()> {
    let lock = read_lock_for_current_platform(root)?;
    if lock.extensions.is_empty() {
        bail!(
            "nothing to install: the lockfile has no extensions. Run `pnl install <source>` to add one."
        );
    }

    let entries = lock
        .extensions
        .values()
        .map(|extension| (extension.source.url.clone(), extension.version.clone()))
        .collect::<Vec<_>>();

    crate::ui::heading("pnl", "install (restore from lockfile)");
    let started = std::time::Instant::now();
    let mut state = InstallState::default();
    for (url, version) in &entries {
        crate::ui::step(&format!("{url} ({version})"));
        // Keep the source's branch; only assert the resolved version matches the lock.
        install_one(
            root,
            manifest,
            url,
            None,
            Some(version),
            options,
            &mut state,
            None,
        )?;
    }

    crate::ui::summary(&format!(
        "restored {} extension(s) in {}",
        entries.len(),
        crate::ui::elapsed(started.elapsed())
    ));
    Ok(())
}

/// The repositories consulted for bare-name resolution, highest priority first.
/// The built-in default repository is appended as the lowest-priority fallback
/// unless the manifest already lists it.
pub(super) fn resolved_repositories(manifest: &PnlManifest) -> Vec<Repository> {
    fn same_url(a: &str, b: &str) -> bool {
        a.trim_end_matches('/') == b.trim_end_matches('/')
    }

    let default_packages = manifest.config.packages_repository();
    let mut repositories = manifest.repositories.clone();
    if !repositories
        .iter()
        .any(|repository| same_url(&repository.url, &default_packages))
    {
        repositories.push(Repository {
            kind: RepositoryType::Git,
            url: default_packages,
            key: None,
            priority: Some(0),
        });
    }

    // Stable sort by priority (default 0) descending: configured repositories of
    // equal priority keep their order and stay ahead of the appended default.
    repositories.sort_by_key(|repository| std::cmp::Reverse(repository.priority.unwrap_or(0)));
    repositories
}

/// A bare package leaf name (e.g. `widget`, `widget-1.0`) — no URL scheme,
/// path separator, or `git@` host — to be resolved against the repositories.
pub(super) fn is_bare_package_name(target: &str) -> bool {
    !target.is_empty()
        && !target.contains("://")
        && !target.contains('/')
        && !target.contains('\\')
        && !target.contains('@')
        && target
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// Resolve a bare package name by appending it to each configured repository URL
/// and installing the first one that exists.
#[allow(clippy::too_many_arguments)]
fn install_bare_name(
    root: &Path,
    manifest: &mut PnlManifest,
    name: &str,
    version_constraint: Option<&str>,
    git_ref: Option<&str>,
    expected_version: Option<&str>,
    options: &InstallOptions,
    state: &mut InstallState,
) -> Result<()> {
    // Configured repositories first (highest priority first), then the built-in
    // default repository as the lowest-priority fallback.
    let repositories = resolved_repositories(manifest);

    let mut failures = Vec::new();
    for repository in &repositories {
        crate::ui::step(&format!("resolving {name} from {}", repository.url));
        match install_from_repository_index(
            root,
            manifest,
            repository,
            name,
            version_constraint,
            options,
            state,
        ) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                failures.push(format!("  - {}: {error}", repository.url));
                continue;
            }
        }

        let candidate = format!("{}/{name}", repository.url.trim_end_matches('/'));
        match install_one(
            root,
            manifest,
            &candidate,
            git_ref,
            expected_version,
            options,
            state,
            None,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => failures.push(format!("  - {}: {error}", repository.url)),
        }
    }

    bail!(
        "could not find package \"{name}\" in any configured repository:\n{}",
        failures.join("\n")
    );
}

fn install_from_repository_index(
    root: &Path,
    manifest: &mut PnlManifest,
    repository: &Repository,
    name: &str,
    version_constraint: Option<&str>,
    options: &InstallOptions,
    state: &mut InstallState,
) -> Result<bool> {
    let Some(index) = load_repository_index(repository)? else {
        return Ok(false);
    };
    let Some((version, entry)) = select_package_version(&index, name, version_constraint)? else {
        return Ok(false);
    };
    crate::ui::success(&format!(
        "selected {name} {version} from repository metadata"
    ));
    install_one(
        root,
        manifest,
        &entry.source.url,
        None,
        Some(&version),
        options,
        state,
        Some(&entry.dist.sha256),
    )?;
    Ok(true)
}

fn version_constraint_for_expected(expected_version: Option<&str>) -> Option<String> {
    expected_version.map(|version| format!("={version}"))
}

/// The `installation` keys tried for the current platform, most specific
/// first. On Linux the distro ID from /etc/os-release (e.g. `alpine`,
/// `ubuntu`, `fedora`) is tried before each `ID_LIKE` ancestor (e.g. `debian`,
/// `rhel`) and the generic `linux` fallback.
fn installation_key_candidates() -> Vec<String> {
    match std::env::consts::OS {
        "macos" => vec!["darwin".to_owned()],
        "linux" => {
            let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            linux_installation_keys(&os_release)
        }
        other => vec![other.to_owned()],
    }
}

/// Candidate keys from /etc/os-release content: `ID`, then the `ID_LIKE`
/// tokens, then `linux`.
fn linux_installation_keys(os_release: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut id_like = Vec::new();
    for line in os_release.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            keys.push(unquote_os_release(value).to_owned());
        } else if let Some(value) = line.strip_prefix("ID_LIKE=") {
            id_like = unquote_os_release(value)
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect();
        }
    }
    keys.extend(id_like);
    keys.push("linux".to_owned());
    keys.dedup();
    keys
}

fn unquote_os_release(value: &str) -> &str {
    value.trim().trim_matches(|ch| ch == '"' || ch == '\'')
}

/// Run a shell command line, inheriting stdio so the user sees its output.
fn run_shell(command_line: &str) -> std::io::Result<std::process::ExitStatus> {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C");
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg("-c");
        c
    };
    command.arg(command_line).status()
}

fn run_self_build(extension_root: &Path, script: &str) -> Result<()> {
    let script = crate::install_script::resolve_package_relative_path(extension_root, script)?;
    crate::ui::step(&format!("running self_build: {}", script.display()));
    let status = if cfg!(windows) {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(&script)
            .current_dir(extension_root)
            .status()
    } else {
        std::process::Command::new("sh")
            .arg(&script)
            .current_dir(extension_root)
            .status()
    }
    .with_context(|| format!("failed to run self_build script {}", script.display()))?;

    if !status.success() {
        bail!("self_build script {} failed ({status})", script.display());
    }
    Ok(())
}

/// If the package declares `installation` for this platform, optionally run its
/// install commands. A passing `checkIfExists` short-circuits (already present);
/// otherwise the user confirms (auto-yes under `--yes`/`--no-interaction`).
fn maybe_install_native_dependencies(
    extension: &PnlxManifest,
    interaction: &crate::interaction::Interaction,
) -> Result<()> {
    let candidates = installation_key_candidates();
    let Some(entry) = candidates
        .iter()
        .find_map(|key| extension.installation.get(key))
    else {
        return Ok(());
    };

    if !entry.check_if_exists.is_empty()
        && entry.check_if_exists.iter().all(|cmd| {
            run_shell(cmd)
                .map(|status| status.success())
                .unwrap_or(false)
        })
    {
        crate::ui::info(&format!(
            "{} native dependencies already present",
            extension.name
        ));
        return Ok(());
    }

    let listed = entry
        .install
        .iter()
        .map(|cmd| format!("    {cmd}"))
        .collect::<Vec<_>>()
        .join("\n");
    let proceed = interaction.confirm(
        &format!(
            "{} needs native dependencies. Run the following to install them?\n{listed}\n",
            extension.name
        ),
        true,
    )?;
    if !proceed {
        crate::ui::warn("skipped native dependency installation; resolution may fail");
        return Ok(());
    }

    for cmd in &entry.install {
        crate::ui::step(&format!("running: {cmd}"));
        let status =
            run_shell(cmd).with_context(|| format!("failed to run install command: {cmd}"))?;
        if !status.success() {
            bail!(
                "tried to install the native dependencies of {name} with the following command, but it failed ({status}):\n    {cmd}\n  install the required libraries and headers manually, then run `pnl install {name}` again",
                name = extension.name,
            );
        }
    }

    Ok(())
}

/// Split a trailing `@<version>` pin off an install target. The version must
/// start with a digit so host parts like `git@github.com` are left untouched.
fn split_version_pin(target: &str) -> (&str, Option<&str>) {
    if let Some((source, version)) = target.rsplit_once('@')
        && !version.is_empty()
        && version.starts_with(|ch: char| ch.is_ascii_digit())
    {
        return (source, Some(version));
    }
    (target, None)
}

#[derive(Debug, Clone)]
enum InstallSource {
    Local { path: PathBuf, source_url: String },
    Git(GitSource),
}

#[derive(Debug, Clone)]
enum ExtensionSource {
    File {
        source_url: String,
    },
    Git {
        source_url: String,
        reference: String,
        dist_url: String,
    },
}

impl ExtensionSource {
    /// The URL the package was installed from (used to match the authorized
    /// repository whitelist).
    fn source_url(&self) -> &str {
        match self {
            ExtensionSource::File { source_url } | ExtensionSource::Git { source_url, .. } => {
                source_url
            }
        }
    }
}

fn is_trusted_extension_source(source: &ExtensionSource, extension_root: &Path) -> bool {
    if crate::config::is_authorized_repository(source.source_url()) {
        return true;
    }

    if matches!(source, ExtensionSource::File { .. })
        && let Some(remote_url) = local_git_origin_url(extension_root)
    {
        return crate::config::is_authorized_repository(&remote_url);
    }

    false
}

fn local_git_origin_url(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?;
    let url = url.trim();
    (!url.is_empty()).then(|| url.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn install_git_extension(
    root: &Path,
    manifest: &mut PnlManifest,
    target: &str,
    source: GitSource,
    expected_version: Option<&str>,
    options: &InstallOptions,
    state: &mut InstallState,
    expected_content_hash: Option<&str>,
) -> Result<()> {
    let installed = install_git_source(&source)?;
    let extension_root = installed.destination.join(&source.package_path);
    if !extension_root
        .join(crate::config::PNLX_MANIFEST_FILE)
        .is_file()
    {
        bail!(
            "git source {} does not contain pnlx.json at the requested package path",
            source.url
        );
    }

    install_local_extension(
        root,
        manifest,
        &extension_root,
        ExtensionSource::Git {
            source_url: target.to_owned(),
            reference: installed.revision.clone(),
            dist_url: target.to_owned(),
        },
        expected_version,
        options,
        state,
        expected_content_hash,
    )
}

fn resolve_install_source(root: &Path, target: &str) -> Result<InstallSource> {
    // Archive distributions (.tar.gz/.tgz/.tar/.zip), local or remote: fetch if
    // needed, extract, and install from the directory that holds pnlx.json.
    if is_archive_source(target) {
        let archive = if is_remote_source(target) {
            fetch_asset(target).with_context(|| format!("failed to download archive {target}"))?
        } else {
            let path = absolutize(root, Path::new(target));
            if !path.is_file() {
                bail!("archive {target} does not exist");
            }
            path
        };
        let extension_root = extract_extension_archive(&archive)?;
        return Ok(InstallSource::Local {
            source_url: target.to_owned(),
            path: extension_root,
        });
    }

    if target.starts_with("ftp://") || target.starts_with("ftps://") {
        bail!(
            "ftp install sources are not implemented yet; use a local path, file:// URL, or git URL"
        );
    }

    if let Some(path) = path_from_file_url(target) {
        let path = absolutize(root, &path);
        ensure_extension_source_path(&path, target)?;
        return Ok(InstallSource::Local {
            source_url: file_url_for_path(&path),
            path,
        });
    }

    let local_target = absolutize(root, Path::new(target));
    if local_target
        .join(crate::config::PNLX_MANIFEST_FILE)
        .is_file()
    {
        return Ok(InstallSource::Local {
            source_url: file_url_for_path(&local_target),
            path: local_target,
        });
    }

    Ok(InstallSource::Git(GitSource::parse(target)?))
}

fn path_from_file_url(value: &str) -> Option<PathBuf> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

fn ensure_extension_source_path(path: &Path, original: &str) -> Result<()> {
    if path.join(crate::config::PNLX_MANIFEST_FILE).is_file() {
        Ok(())
    } else {
        bail!("{original} does not point to an extension root containing pnlx.json")
    }
}

#[allow(clippy::too_many_arguments)]
fn install_local_extension(
    root: &Path,
    manifest: &mut PnlManifest,
    extension_root: &Path,
    source: ExtensionSource,
    expected_version: Option<&str>,
    options: &InstallOptions,
    state: &mut InstallState,
    expected_content_hash: Option<&str>,
) -> Result<()> {
    let mut extension =
        read_json::<PnlxManifest>(&extension_root.join(crate::config::PNLX_MANIFEST_FILE))?;
    validate_schema_version(&extension.schema_version)?;
    validate_pnlx_manifest_values(&extension)?;
    // Canonicalize a two-part upstream version (e.g. pcre2 `10.43`) so the path it
    // installs to, the lockfile, and the `=<version>` pin in `pnl.json` are all valid
    // three-part semver.
    extension.version = crate::version::to_canonical_semver(&extension.version);

    // Refuse early (before running any install scripts) when the package does
    // not declare support for this platform — e.g. a library that is not
    // packaged for Alpine/musl.
    let current = crate::platform::current_platform_requirement();
    if !crate::platform::platform_supported(&extension.platforms, &current) {
        let supported = extension
            .platforms
            .iter()
            .map(crate::platform::describe_platform)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} {} does not support this platform ({}); supported platforms: {}",
            extension.name,
            extension.version,
            crate::platform::describe_platform(&current),
            supported
        );
    }

    // Enforce an `@<version>` pin or a lockfile restore against the resolved version.
    // Both sides are canonicalized so a two-part pin (`@10.43`) matches `10.43.0`.
    if let Some(expected) = expected_version
        && extension.version != crate::version::to_canonical_semver(expected)
    {
        bail!(
            "expected {} version {expected}, but the resolved package is version {}",
            extension.name,
            extension.version
        );
    }

    // Integrity signature: hash the package content and reject it if a prior
    // lock pinned the same version to a different digest (tampered download).
    let content_hash = tree_sha256(extension_root)?;
    if let Some(expected) = expected_content_hash
        && expected != content_hash
    {
        bail!(
            "dist integrity check failed for {name}: repository index expected sha256 {expected}, but resolved content is {content_hash}",
            name = extension.name
        );
    }
    verify_locked_integrity(root, &extension, &content_hash, options.force)?;
    crate::install_script::verify_install_scripts(
        extension_root,
        &extension,
        &options.interaction,
        options.allow_unverified_install_scripts,
        &options.allowed_install_script_hashes,
        is_trusted_extension_source(&source, extension_root),
    )?;

    ensure_not_cyclic(state, &extension.name)?;
    state.stack.push(extension.name.clone());

    // A nested dependency install (writing into a parent's subtree) is private to
    // that parent: it is not a top-level entity, so it stays out of `pnl.json` and
    // the top-level lock, and each parent keeps its own (possibly different) version.
    let is_dependency = state.is_dependency();
    let packages_root = current_packages_root(root, state);

    // Install this package's own files first, so dependency packages can be placed
    // *inside* its subtree without the file copy clobbering them.
    let installed_extension_root =
        install_extension_files(&packages_root, extension_root, &extension.name, &extension.version)?;

    // Dependency packages install nested under this package's own directory.
    let previous_packages_root = state
        .packages_root
        .replace(installed_extension_root.join("packages"));
    install_extension_dependencies(root, manifest, &extension, options, state)?;
    state.packages_root = previous_packages_root;

    if !is_dependency {
        manifest
            .extensions
            .entry(extension.name.clone())
            .or_insert_with(|| ExtensionRequirement {
                version: format!("={}", extension.version),
                required: true,
            });
        write_json(&root.join(crate::config::PNL_MANIFEST_FILE), manifest)?;
    }

    // Offer to install the package's native dependencies (e.g. `brew install …`)
    // before we try to resolve them from disk.
    if let Some(script) = &extension.self_build {
        run_self_build(&installed_extension_root, script)?;
    } else {
        maybe_install_native_dependencies(&extension, &options.interaction)?;
    }

    let mut lock = read_lock_for_current_platform(root)?;
    lock.generated_at = now();
    // Resolve the package's build-time `require_definitions`, preseeded from the
    // prior solved values in the lock so a reinstall keeps the earlier choice and a
    // non-interactive install reproduces it.
    let prior_definitions = lock
        .extensions
        .get(&extension.name)
        .map(|locked| locked.definitions.clone())
        .unwrap_or_default();
    let resolved_definitions = resolve_require_definitions(
        &extension.name,
        &extension.require_definitions,
        &prior_definitions,
        &options.interaction,
    )?;
    let mut locked_requires = BTreeMap::new();
    let mut pathmap = read_pathmap_for_current_platform(root)?;
    pathmap.generated_at = now();

    // The current architecture's dependency packages (to map their C functions) and
    // co-load libraries (extra `.so` whose symbols the package's own calls resolve
    // against, e.g. gsl -> cblas, brotli -> brotlicommon).
    let dependency_arch_entries = extension.dependencies_for_current_arch();
    let dependency_package_names: Vec<String> = dependency_arch_entries
        .iter()
        .flat_map(|entry| entry.package_names.iter().cloned())
        .collect();
    let dependency_libraries =
        resolve_dependency_libraries(root, manifest, &dependency_arch_entries)?;

    // Map a (recursive) dependency package's C functions to its entity class, so a
    // function-like macro that calls one resolves to that class instead of becoming
    // a thrower. The dependencies were installed (and locked) above.
    let dependency_functions =
        collect_dependency_functions(&installed_extension_root.join("packages"), &dependency_package_names);
    for (key, requirement) in &extension.requires {
        let mut native = resolve_native_library(root, manifest, key, requirement)?;
        // Stamp the first-install time, preserving it across reinstalls so the
        // timestamp reflects when the library first entered the workspace.
        native.installed_at = pathmap
            .requires
            .get(key)
            .and_then(|previous| previous.installed_at.clone())
            .or_else(|| Some(now()));
        crate::ui::success(&format!(
            "resolved {key} {} {}",
            native.version,
            crate::ui::dim(&native.resolved_name)
        ));
        let header = resolve_header_for_native(
            extension_root,
            &installed_extension_root,
            &native.path,
            key,
            requirement,
        )?;
        let generation_headers =
            generation_headers_from_resolved_header(&header, &requirement.header_names);
        let header_include_dirs: Vec<std::path::PathBuf> = header
            .include_dirs
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        locked_requires.insert(
            key.clone(),
            LockedNativeLibrary {
                name: native.resolved_name.clone(),
                version: native.version.clone(),
                sha256: native.sha256.clone(),
            },
        );
        let native_path = native.path.clone();
        let native_sha256 = native.sha256.clone();
        // Limit the generated cdef to symbols the installed libraries actually
        // export, so version/build skew (a declared-but-absent function) does not
        // fail the whole FFI load. With co-load libraries the set is the *union* of
        // the package's own library and every co-load library, so a function exported
        // by a dependency (e.g. a second `.so` in the same package) is kept and stays
        // callable through the one monolithic cdef.
        let exported = exported_symbols_union(&native_path, &dependency_libraries);
        pathmap.requires.insert(key.clone(), native);
        pathmap.headers.insert(key.clone(), header);
        generate_installed_package_artifacts(
            &installed_extension_root,
            &extension,
            extension.name.rsplit('/').next().unwrap_or(key),
            key,
            &generation_headers,
            &header_include_dirs,
            options.alias_class.as_deref(),
            options.function_prefix.as_deref(),
            &dependency_functions,
            exported.as_ref(),
            &resolved_definitions,
        )?;
        if let Some(fqcn) = entity_class_fqn(&extension) {
            // Bake the resolved native library path + hash + co-load library paths
            // into the entity constants.
            let class_name = fqcn.rsplit('\\').next().unwrap_or(&fqcn);
            let library_paths: Vec<String> = dependency_libraries.values().cloned().collect();
            crate::generate::stamp_entity_native_library(
                &installed_extension_root.join(crate::config::GENERATED_DIR),
                class_name,
                &native_path,
                &native_sha256,
                &library_paths,
            )?;
        }
    }

    // A nested dependency is private to its parent: it is recorded in the parent's
    // `LockedExtension.dependencies`, not as its own top-level lock entity (so two
    // parents can pin different versions). Only top-level installs are locked here.
    if !is_dependency {
    let (source, dist) = source.lock_source(&extension, &content_hash);
    lock.extensions.insert(
        extension.name.clone(),
        LockedExtension {
            version: extension.version.clone(),
            constraint: format!("={}", extension.version),
            source,
            dist,
            classes: entity_class_fqn(&extension).into_iter().collect(),
            // Dependency packages (name -> resolved version), used for recursive
            // function mapping. Co-load libraries are recorded separately below.
            dependencies: dependency_package_names
                .iter()
                .map(|name| {
                    let version = lock
                        .extensions
                        .get(name)
                        .map(|locked| locked.version.clone())
                        .unwrap_or_else(|| "*".to_owned());
                    (name.clone(), version)
                })
                .collect(),
            requires: locked_requires,
            // Resolved co-load libraries (name -> path), so a reinstall and the
            // runtime know exactly which extra `.so` to load alongside this package.
            libraries: dependency_libraries.clone(),
            definitions: resolved_definitions
                .iter()
                .map(|definition| (definition.name.clone(), definition.value.clone()))
                .collect(),
        },
    );
    write_json(&pnl_lock_path(root), &lock)?;
    }

    write_pathmap(root, &pathmap)?;
    write_pnlx_autoload(root)?;
    crate::ui::success(&format!("installed extension {}", extension.name));

    // Show the package's optional usage examples — package-relative files
    // (e.g. EXAMPLES.md) — so users see how to call it.
    for (index, example) in extension.examples.iter().enumerate() {
        let label = if extension.examples.len() > 1 {
            format!("example {} — {} ({example})", index + 1, extension.name)
        } else {
            format!("usage — {} ({example})", extension.name)
        };
        let path = installed_extension_root.join(example);
        match std::fs::read_to_string(&path) {
            Ok(body) => crate::ui::example_block(&label, body.trim_end()),
            Err(_) => crate::ui::warn(&format!(
                "example file {example} is missing from the {} package",
                extension.name
            )),
        }
    }

    Ok(())
}

fn ensure_not_cyclic(state: &InstallState, package: &str) -> Result<()> {
    if state.stack.iter().any(|item| item == package) {
        let mut cycle = state.stack.clone();
        cycle.push(package.to_owned());
        bail!("cyclic dependency detected: {}", cycle.join(" -> "));
    }
    Ok(())
}

fn install_extension_dependencies(
    root: &Path,
    manifest: &mut PnlManifest,
    extension: &PnlxManifest,
    options: &InstallOptions,
    state: &mut InstallState,
) -> Result<()> {
    // The `package_names` of the current architecture's dependency entries are other
    // pnl packages to install and co-load. A bare name resolves through the
    // registries (the entry's `repositories`, or the workspace/config defaults); a
    // `file://`/`git@`/path entry resolves like an `install` target.
    for entry in extension.dependencies_for_current_arch() {
        let added = extend_manifest_repositories(manifest, &entry.repositories);
        for package in &entry.package_names {
            if installed_dependency_satisfies(root, package, "*").unwrap_or(false) {
                crate::ui::info(&format!("dependency {package} already installed"));
                continue;
            }
            crate::ui::step(&format!("installing dependency {package}"));
            install_target(root, manifest, package, options, state)?;
        }
        truncate_manifest_repositories(manifest, added);
    }
    let _ = state.stack.pop();
    Ok(())
}

/// Append a dependency entry's `repositories` (URLs not already present) to the
/// workspace manifest so a bare `package_names` entry resolves against them, and
/// return how many were added (to undo afterwards).
fn extend_manifest_repositories(manifest: &mut PnlManifest, repositories: &[String]) -> usize {
    let mut added = 0;
    for url in repositories {
        if manifest.repositories.iter().any(|repo| &repo.url == url) {
            continue;
        }
        manifest.repositories.push(Repository {
            kind: repository_kind_for_url(url),
            url: url.clone(),
            key: None,
            // Consulted before the built-in default (priority 0).
            priority: Some(1),
        });
        added += 1;
    }
    added
}

/// Guess a repository's type from its URL scheme (a `git`/`.git` URL is Git, a
/// `file://`/path is File, otherwise an HTTPS index).
fn repository_kind_for_url(url: &str) -> RepositoryType {
    if url.starts_with("git@") || url.ends_with(".git") || url.starts_with("git://") {
        RepositoryType::Git
    } else if url.starts_with("file://") || url.starts_with('/') || url.starts_with('.') {
        RepositoryType::File
    } else {
        RepositoryType::Https
    }
}

/// Undo the temporary repositories added by [`extend_manifest_repositories`].
fn truncate_manifest_repositories(manifest: &mut PnlManifest, added: usize) {
    let keep = manifest.repositories.len().saturating_sub(added);
    manifest.repositories.truncate(keep);
}

/// Install a `package_names` entry. `install_one` already resolves a bare name (via
/// the registries), a `file://`/path, or a `git@`/git URL.
fn install_target(
    root: &Path,
    manifest: &mut PnlManifest,
    target: &str,
    options: &InstallOptions,
    state: &mut InstallState,
) -> Result<()> {
    install_one(root, manifest, target, None, None, options, state, None)
}

fn installed_dependency_satisfies(root: &Path, package: &str, constraint: &str) -> Result<bool> {
    let lock = read_lock_for_current_platform(root)?;
    let Some(installed) = lock.extensions.get(package) else {
        return Ok(false);
    };
    installed_version_satisfies(&installed.version, constraint)
}

impl ExtensionSource {
    fn lock_source(&self, extension: &PnlxManifest, content_hash: &str) -> (Source, Dist) {
        match self {
            Self::File { source_url } => (
                Source {
                    kind: RepositoryType::File,
                    url: source_url.clone(),
                    reference: extension.version.clone(),
                },
                Dist {
                    url: source_url.clone(),
                    sha256: content_hash.to_owned(),
                },
            ),
            Self::Git {
                source_url,
                reference,
                dist_url,
            } => (
                Source {
                    kind: RepositoryType::Git,
                    url: source_url.clone(),
                    reference: reference.clone(),
                },
                Dist {
                    url: dist_url.clone(),
                    sha256: content_hash.to_owned(),
                },
            ),
        }
    }
}

/// Build a `C function name -> dependency entity FQCN` map by walking a package's
/// `dependencies` (recursively, through the lockfile). Each installed dependency
/// contributes its locked entity class for every C function in its generated
/// cdef, so a function-like macro that calls one can render a static call to it.
/// Resolve the current architecture's `library_names` dependency entries to extra
/// shared libraries to co-load, as `resolved name -> resolved path`. Each group is
/// resolved with the same path logic as a native requirement (pkg-config / soname /
/// multiarch). `package_names` are handled by the package-install path, not here.
fn resolve_dependency_libraries(
    root: &Path,
    manifest: &PnlManifest,
    entries: &[&crate::manifest::DependencyEntry],
) -> Result<BTreeMap<String, String>> {
    let mut resolved = BTreeMap::new();
    for entry in entries {
        if entry.library_names.is_empty() {
            continue;
        }
        let requirement = crate::manifest::NativeRequirement {
            library_names: entry.library_names.clone(),
            header_names: Vec::new(),
            symbol_prefix: None,
            library_url: None,
            header_url: None,
            header_inline: None,
            version: ">=0.0.0".to_owned(),
            required: true,
        };
        // The first real (non-virtual) name's stem is the pkg-config lookup key.
        let key = entry
            .library_names
            .iter()
            .find(|name| !name.is_virtual())
            .map(|name| library_stem(name.name()))
            .unwrap_or_default();
        let native = resolve_native_library(root, manifest, &key, &requirement)?;
        resolved.insert(native.resolved_name, native.path);
    }
    Ok(resolved)
}

/// The union of the symbols exported by the package's own library and every
/// co-load dependency library, for the cdef's export filter. `None` (no filter)
/// when the package's own exports can't be read, preserving the prior behaviour;
/// an unreadable dependency just contributes nothing.
fn exported_symbols_union(
    primary_path: &str,
    dependency_libraries: &BTreeMap<String, String>,
) -> Option<std::collections::BTreeSet<String>> {
    let mut union = crate::commands::pnl::native::exported_symbols(primary_path)?;
    for path in dependency_libraries.values() {
        if let Some(symbols) = crate::commands::pnl::native::exported_symbols(path) {
            union.extend(symbols);
        }
    }
    Some(union)
}

/// The pkg-config-style stem of a library file name (`libgslcblas.so.0` ->
/// `gslcblas`): drop a leading `lib` and everything from the first `.`.
fn library_stem(name: &str) -> String {
    let base = name.split('.').next().unwrap_or(name);
    base.strip_prefix("lib").unwrap_or(base).to_owned()
}

fn collect_dependency_functions(
    packages_root: &Path,
    package_names: &[String],
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    collect_dependency_functions_in(packages_root, package_names, &mut map, &mut seen);
    map
}

/// Walk the dependency packages nested under `packages_root`, mapping each one's C
/// functions to its entity class, and recurse into each dependency's own nested
/// dependencies. Resolved straight from the nested files (not the lock), since
/// dependencies are private to their parent and not top-level lock entities.
fn collect_dependency_functions_in(
    packages_root: &Path,
    package_names: &[String],
    map: &mut BTreeMap<String, String>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    for name in package_names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(version_dir) = sole_installed_version_dir(packages_root, name) else {
            continue;
        };
        let Ok(manifest) =
            read_json::<PnlxManifest>(&version_dir.join(crate::config::PNLX_MANIFEST_FILE))
        else {
            continue;
        };
        if let Some(fqcn) = entity_class_fqn(&manifest) {
            // The dependency's entity class, made absolute for a static `::` call.
            let fqcn = format!("\\{}", fqcn.trim_start_matches('\\'));
            let generated = version_dir.join(crate::config::GENERATED_DIR);
            if let Ok(entries) = std::fs::read_dir(&generated) {
                for path in entries.flatten().map(|entry| entry.path()) {
                    if !path
                        .file_name()
                        .and_then(|file| file.to_str())
                        .is_some_and(|file| file.ends_with(crate::config::FFI_FILE_SUFFIX))
                    {
                        continue;
                    }
                    if let Ok(cdef) = read_existing_ffi_cdef(&path) {
                        for signature in parse_function_signatures(&cdef) {
                            map.entry(signature.name).or_insert_with(|| fqcn.clone());
                        }
                    }
                }
            }
        }
        // Recurse into the dependency's own nested dependency packages.
        let nested_names: Vec<String> = manifest
            .dependencies_for_current_arch()
            .iter()
            .flat_map(|entry| entry.package_names.iter().cloned())
            .collect();
        collect_dependency_functions_in(
            &version_dir.join("packages"),
            &nested_names,
            map,
            seen,
        );
    }
}

/// The sole installed version directory of `package` under `packages_root` — each
/// nested dependency has exactly one version installed.
fn sole_installed_version_dir(packages_root: &Path, package: &str) -> Option<std::path::PathBuf> {
    let package_dir = package_dir_in(packages_root, package);
    std::fs::read_dir(&package_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join(crate::config::PNLX_MANIFEST_FILE).is_file())
}

/// Reject an install whose content digest differs from a previously locked
/// digest for the *same* version — the hallmark of tampered-with content. A new
/// version is treated as a legitimate update and is allowed through.
fn verify_locked_integrity(
    root: &Path,
    extension: &PnlxManifest,
    content_hash: &str,
    force: bool,
) -> Result<()> {
    let lock = read_lock_for_current_platform(root)?;
    let Some(existing) = lock.extensions.get(&extension.name) else {
        return Ok(());
    };

    if existing.version == extension.version && existing.dist.sha256 != content_hash {
        if force {
            // `--force`: trust the resolved content and let the caller overwrite
            // the locked digest instead of aborting.
            crate::ui::warn(&format!(
                "{name}: content does not match the lockfile digest; overwriting it because --force was given\n  \
                 was sha256: {expected}\n  \
                 now sha256: {actual}",
                name = extension.name,
                expected = existing.dist.sha256,
                actual = content_hash,
            ));
            return Ok(());
        }
        bail!(
            "integrity check failed for {name}: the content does not match the signature recorded in the lockfile.\n  \
             expected sha256: {expected}\n  \
             actual sha256:   {actual}\n\
             The package content may have been modified or tampered with; aborting install.\n\
             If this change is intentional, bump the version or remove the {name} entry from {lock} and reinstall (or pass --force).",
            name = extension.name,
            expected = existing.dist.sha256,
            actual = content_hash,
            lock = pnl_lock_path(root).display(),
        );
    }

    crate::ui::success(&format!("verified {} integrity", extension.name));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::package::tree_sha256;
    use super::{
        ExtensionSource, InstallSource, is_trusted_extension_source, path_from_file_url,
        resolve_install_source, split_version_pin,
    };

    fn int_definition(default: Option<serde_json::Value>) -> Vec<crate::manifest::RequireDefinition> {
        vec![crate::manifest::RequireDefinition {
            name: "WIDTH".to_owned(),
            description: String::new(),
            definition_type: crate::manifest::DefinitionType::Int,
            default,
        }]
    }

    #[test]
    fn require_definitions_use_default_when_non_interactive() {
        let definitions = int_definition(Some(serde_json::json!(8)));
        let interaction = crate::interaction::Interaction::new(true, false);
        let resolved = super::resolve_require_definitions(
            "vendor/pkg",
            &definitions,
            &std::collections::BTreeMap::new(),
            &interaction,
        )
        .unwrap();
        assert_eq!(resolved[0].value, "8");
    }

    #[test]
    fn require_definitions_prefer_the_locked_value_over_the_default() {
        let definitions = int_definition(Some(serde_json::json!(8)));
        let prior = std::collections::BTreeMap::from([("WIDTH".to_owned(), "16".to_owned())]);
        let interaction = crate::interaction::Interaction::new(true, false);
        let resolved =
            super::resolve_require_definitions("vendor/pkg", &definitions, &prior, &interaction)
                .unwrap();
        assert_eq!(resolved[0].value, "16");
    }

    #[test]
    fn require_definitions_error_when_unresolvable_and_non_interactive() {
        let definitions = int_definition(None);
        let interaction = crate::interaction::Interaction::new(true, false);
        let result = super::resolve_require_definitions(
            "vendor/pkg",
            &definitions,
            &std::collections::BTreeMap::new(),
            &interaction,
        );
        assert!(result.is_err(), "expected an error for an unresolvable definition");
    }

    #[test]
    fn library_stem_drops_lib_prefix_and_version_suffix() {
        assert_eq!(super::library_stem("libgslcblas.so.0"), "gslcblas");
        assert_eq!(super::library_stem("libbrotlicommon.so"), "brotlicommon");
        assert_eq!(super::library_stem("foo"), "foo");
    }

    #[test]
    fn dependency_entries_match_current_arch_and_wildcard() {
        use crate::manifest::{DependencyEntry, LibraryName};
        let arch = crate::platform::current_platform_requirement().arch;
        let entry = |name: &str| DependencyEntry {
            library_names: vec![LibraryName::Plain(name.to_owned())],
            ..Default::default()
        };
        let mut manifest = crate::manifest::PnlxManifest::default();
        manifest.dependencies.insert(arch, vec![entry("libfoo.so")]);
        manifest
            .dependencies
            .insert("any".to_owned(), vec![entry("libbar.so")]);
        manifest
            .dependencies
            .insert("some-other-arch".to_owned(), vec![entry("libnope.so")]);

        let names: Vec<String> = manifest
            .dependencies_for_current_arch()
            .iter()
            .flat_map(|entry| entry.library_names.iter().map(|name| name.name().to_owned()))
            .collect();
        assert!(names.contains(&"libfoo.so".to_owned()), "{names:?}");
        assert!(names.contains(&"libbar.so".to_owned()), "{names:?}");
        assert!(!names.contains(&"libnope.so".to_owned()), "{names:?}");
    }

    #[test]
    fn resolves_linux_installation_keys_from_os_release() {
        use super::linux_installation_keys;
        assert_eq!(
            linux_installation_keys("ID=ubuntu\nID_LIKE=debian\n"),
            vec!["ubuntu", "debian", "linux"]
        );
        assert_eq!(
            linux_installation_keys("ID=alpine\n"),
            vec!["alpine", "linux"]
        );
        assert_eq!(
            linux_installation_keys("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"),
            vec!["rocky", "rhel", "centos", "fedora", "linux"]
        );
        // No /etc/os-release (or an unreadable one) still falls back to linux.
        assert_eq!(linux_installation_keys(""), vec!["linux"]);
    }

    #[test]
    fn recognizes_bare_package_names() {
        use super::is_bare_package_name;
        assert!(is_bare_package_name("widget"));
        assert!(is_bare_package_name("widget-1.0"));
        assert!(!is_bare_package_name("vendor/pkg"));
        assert!(!is_bare_package_name("./pkg"));
        assert!(!is_bare_package_name("https://example.com/pkg"));
        assert!(!is_bare_package_name("git@github.com:o/r.git"));
        assert!(!is_bare_package_name(""));
    }

    #[test]
    fn splits_trailing_version_pin_but_not_host_at() {
        assert_eq!(
            split_version_pin("https://github.com/o/widget@2.32.10"),
            ("https://github.com/o/widget", Some("2.32.10"))
        );
        assert_eq!(
            split_version_pin("git@github.com:o/repo@1.2.3"),
            ("git@github.com:o/repo", Some("1.2.3"))
        );
        // A bare scp host must not be mistaken for a version pin.
        assert_eq!(
            split_version_pin("git@github.com:o/repo.git"),
            ("git@github.com:o/repo.git", None)
        );
        assert_eq!(split_version_pin("/local/path"), ("/local/path", None));
        assert_eq!(split_version_pin("./pkg"), ("./pkg", None));
    }

    #[test]
    fn tree_hash_is_deterministic_and_content_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("b.txt"), "beta").unwrap();

        let baseline = tree_sha256(dir.path()).unwrap();
        assert_eq!(baseline, tree_sha256(dir.path()).unwrap());

        std::fs::write(dir.path().join("sub").join("b.txt"), "BETA").unwrap();
        assert_ne!(baseline, tree_sha256(dir.path()).unwrap());
    }

    #[test]
    fn integrity_check_rejects_tampered_same_version() {
        use super::verify_locked_integrity;
        use crate::manifest::{Dist, LockedExtension, PnlLock, PnlxManifest, Source};
        use crate::platform::current_platform;
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        let locked_hash = "a".repeat(64);
        let mut lock = PnlLock::empty(current_platform());
        lock.extensions.insert(
            "vendor/example".to_owned(),
            LockedExtension {
                version: "1.0.0".to_owned(),
                constraint: "=1.0.0".to_owned(),
                source: Source {
                    kind: crate::manifest::RepositoryType::File,
                    url: "file:///pkg".to_owned(),
                    reference: "1.0.0".to_owned(),
                },
                dist: Dist {
                    url: "file:///pkg".to_owned(),
                    sha256: locked_hash.clone(),
                },
                classes: Vec::new(),
                dependencies: BTreeMap::new(),
                requires: BTreeMap::new(),
                libraries: BTreeMap::new(),
                definitions: BTreeMap::new(),
            },
        );
        crate::io::write_json(&super::pnl_lock_path(dir.path()), &lock).unwrap();

        let mut extension = PnlxManifest {
            name: "vendor/example".to_owned(),
            version: "1.0.0".to_owned(),
            ..PnlxManifest::default()
        };

        // Same version + matching digest is fine.
        assert!(verify_locked_integrity(dir.path(), &extension, &locked_hash, false).is_ok());
        // Same version + different digest is a tamper and must be rejected.
        assert!(verify_locked_integrity(dir.path(), &extension, &"b".repeat(64), false).is_err());
        // ...unless --force is given, which trusts the resolved content.
        assert!(verify_locked_integrity(dir.path(), &extension, &"b".repeat(64), true).is_ok());
        // A new version is a legitimate update and is allowed through.
        extension.version = "2.0.0".to_owned();
        assert!(verify_locked_integrity(dir.path(), &extension, &"b".repeat(64), false).is_ok());
    }

    #[test]
    fn tree_hash_ignores_generated_and_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(crate::config::PNLX_MANIFEST_FILE), "{}").unwrap();
        let baseline = tree_sha256(dir.path()).unwrap();

        std::fs::create_dir_all(dir.path().join("src").join("generated")).unwrap();
        std::fs::write(
            dir.path().join("src").join("generated").join("x.php"),
            "<?php",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: x").unwrap();

        assert_eq!(baseline, tree_sha256(dir.path()).unwrap());
    }

    #[test]
    fn resolves_absolute_file_url_as_local_install_source() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("extension");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join(crate::config::PNLX_MANIFEST_FILE), "{}").unwrap();

        let source =
            resolve_install_source(temp.path(), &format!("file://{}", package.display())).unwrap();

        match source {
            InstallSource::Local { path, source_url } => {
                assert_eq!(path, package);
                assert!(source_url.starts_with("file://"));
                assert!(source_url.ends_with("/extension"));
            }
            InstallSource::Git(_) => panic!("expected local install source"),
        }
    }

    #[test]
    fn trusts_local_packages_from_authorized_git_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("packages").join("libsdl");
        std::fs::create_dir_all(&package).unwrap();

        let init = std::process::Command::new("git")
            .arg("init")
            .arg(temp.path())
            .output()
            .unwrap();
        assert!(init.status.success());
        let config = std::process::Command::new("git")
            .arg("-C")
            .arg(temp.path())
            .args([
                "config",
                "remote.origin.url",
                "git@github.com:m3m0r7/pnl-packages.git",
            ])
            .output()
            .unwrap();
        assert!(config.status.success());

        let source = ExtensionSource::File {
            source_url: format!("file://{}", package.display()),
        };

        assert!(is_trusted_extension_source(&source, &package));
    }

    #[test]
    fn parses_localhost_file_url_path() {
        assert_eq!(
            path_from_file_url("file://localhost/tmp/pnl-package").unwrap(),
            std::path::PathBuf::from("/tmp/pnl-package")
        );
    }
}
