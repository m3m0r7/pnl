use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::app::commands::pnlx::{
    generate_installed_package_artifacts, manifest_requires_libclang, read_existing_ffi_cdef,
};
use crate::codegen::parse_function_signatures;
use crate::model::manifest::{
    DefinitionType, Dist, ExtensionRequirement, LockedExtension, LockedNativeLibrary, PnlManifest,
    PnlxManifest, Repository, RepositoryType, RequireDefinition, ResolvedDefinition, Source,
};
use crate::model::platform::now;
use crate::model::repository_index::{
    installed_version_satisfies, load_repository_index, select_package_version,
};
use crate::model::validate::{
    validate_pnl_manifest_values, validate_pnlx_manifest_values, validate_schema_version,
};
use crate::sources::archive::{extract_extension_archive, is_archive_source};
use crate::sources::fetch::{fetch_asset, is_remote_source};
use crate::sources::git_source::{GitSource, install_git_source};
use crate::util::io::{read_json, read_or_default, write_json};

use crate::native::discovery::{
    generation_headers_from_resolved_header, resolve_header_for_native, resolve_native_library,
};
use crate::util::path::absolutize;

use super::package::{
    entity_class_fqn, file_url_for_path, install_extension_files, package_dir_in, pnl_lock_path,
    pnlx_workspace_dir, read_lock_for_current_platform, read_pathmap_for_current_platform,
    tree_sha256, write_pathmap, write_pnlx_autoload,
};

mod definitions;
mod native_deps;

use definitions::resolve_require_definitions;
use native_deps::{maybe_install_native_dependencies, run_self_build};

/// Options supplied on the `pnl install` command line.
#[derive(Debug, Clone, Default)]
pub(crate) struct InstallOptions {
    /// Define a `class_alias` for the generated extension class.
    pub alias_class: Option<String>,
    /// Prefix added to every generated function and method name.
    pub function_prefix: Option<String>,
    /// Drives confirmation prompts (e.g. native-dependency installation).
    pub interaction: crate::app::interaction::Interaction,
    /// Continue when install scripts are missing or fail their publish-time hash
    /// check.
    pub allow_unverified_install_scripts: bool,
    /// Hashes explicitly trusted for this install run.
    pub allowed_install_script_hashes: Vec<String>,
    /// Persist `features.global_functions = true` into pnl.json.
    pub enable_global_functions: bool,
    /// Persist `features.cdata_arguments = true` into pnl.json.
    pub enable_cdata_arguments: bool,
    /// Persist `features.scalar_returns = true` into pnl.json.
    pub enable_scalar_returns: bool,
    /// Persist `features.scalar_constants = true` into pnl.json.
    pub enable_scalar_constants: bool,
    /// Persist `compile_options.static_inline = true` into pnl.json (build a
    /// trampoline shim so `static inline` functions become bound methods).
    pub enable_static_inline: bool,
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
    if options.enable_global_functions && !manifest.features.global_functions {
        manifest.features.global_functions = true;
        changed = true;
    }
    if options.enable_cdata_arguments && !manifest.features.cdata_arguments {
        manifest.features.cdata_arguments = true;
        changed = true;
    }
    if options.enable_scalar_returns && !manifest.features.scalar_returns {
        manifest.features.scalar_returns = true;
        changed = true;
    }
    if options.enable_scalar_constants && !manifest.features.scalar_constants {
        manifest.features.scalar_constants = true;
        changed = true;
    }
    if options.enable_static_inline && !manifest.compile_options.static_inline {
        manifest.compile_options.static_inline = true;
        changed = true;
    }
    changed
}

pub(super) fn install(root: &Path, targets: &[String], options: &InstallOptions) -> Result<()> {
    let mut manifest =
        read_or_default::<PnlManifest>(&root.join(crate::model::config::PNL_MANIFEST_FILE))?;
    validate_schema_version(&manifest.schema_version)?;
    validate_pnl_manifest_values(&manifest)?;

    // Persist feature toggles up front so `pnl install --enable-…` works even with
    // no target (the manifest is rewritten again when an extension is added).
    if apply_feature_flags(&mut manifest, options) {
        write_json(
            &root.join(crate::model::config::PNL_MANIFEST_FILE),
            &manifest,
        )?;
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
    crate::app::ui::heading("pnl", &label);
    let started = std::time::Instant::now();
    let mut state = InstallState::default();

    for target in targets {
        // An optional `@<version>` suffix pins the version (e.g. `…/widget@1.2.3`).
        let (target, pinned_version) = split_version_pin(target);
        if targets.len() > 1 {
            crate::app::ui::step(target);
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

    crate::app::ui::summary(&format!(
        "added {} extension(s) in {}",
        targets.len(),
        crate::app::ui::elapsed(started.elapsed())
    ));
    Ok(())
}

/// Offer to add the generated workspace directory (`@pnlx`) to `.gitignore` — it
/// is regenerable and should not be committed. No-op when it is already ignored,
/// the user declines, or the prompt is non-interactive.
pub(super) fn offer_gitignore(
    root: &Path,
    interaction: &crate::app::interaction::Interaction,
) -> Result<()> {
    let output_dir = crate::model::workspace::output_dir_name(root);
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
    crate::app::ui::created("updated", &gitignore);
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
    // A package leaf or `vendor/package` identity (not a local package dir) is
    // resolved against the configured repositories, e.g. `pnl install widget`.
    if is_bare_package_name(target)
        && !absolutize(root, Path::new(target))
            .join(crate::model::config::PNLX_MANIFEST_FILE)
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

    crate::app::ui::heading("pnl", "install (restore from lockfile)");
    let started = std::time::Instant::now();
    let mut state = InstallState::default();
    for (url, version) in &entries {
        crate::app::ui::step(&format!("{url} ({version})"));
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

    crate::app::ui::summary(&format!(
        "restored {} extension(s) in {}",
        entries.len(),
        crate::app::ui::elapsed(started.elapsed())
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

    let default_packages = manifest.config.package_repository();
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

/// A package leaf name (`widget`) or full identity (`vendor/widget`) to resolve
/// against the repositories. An existing local directory containing `pnlx.json`
/// takes precedence in [`install_one`].
pub(super) fn is_bare_package_name(target: &str) -> bool {
    if target.is_empty() || target.contains("://") || target.contains('\\') || target.contains('@')
    {
        return false;
    }

    let segments = target.split('/').collect::<Vec<_>>();
    (segments.len() == 1 || segments.len() == 2)
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && !matches!(*segment, "." | "..")
                && segment
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        })
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
        crate::app::ui::step(&format!("resolving {name} from {}", repository.url));
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
    crate::app::ui::success(&format!(
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
    if crate::model::config::is_authorized_repository(source.source_url()) {
        return true;
    }

    if matches!(source, ExtensionSource::File { .. })
        && let Some(remote_url) = local_git_origin_url(extension_root)
    {
        return crate::model::config::is_authorized_repository(&remote_url);
    }

    false
}

fn local_git_origin_url(path: &Path) -> Option<String> {
    // Read `remote.origin.url` through libgit2 (vendored), not the `git` CLI, so
    // `pnl install` needs no external git binary. `discover` walks up to the repo
    // root, matching `git -C <path>`'s behaviour from a checkout subdirectory.
    let repository = git2::Repository::discover(path).ok()?;
    let origin = repository.find_remote("origin").ok()?;
    // `url()` errors only when the configured URL is not valid UTF-8; treat that as
    // "no usable origin" like the other failure paths.
    origin.url().ok().map(str::to_owned)
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
        .join(crate::model::config::PNLX_MANIFEST_FILE)
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
        .join(crate::model::config::PNLX_MANIFEST_FILE)
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
    if path
        .join(crate::model::config::PNLX_MANIFEST_FILE)
        .is_file()
    {
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
        read_json::<PnlxManifest>(&extension_root.join(crate::model::config::PNLX_MANIFEST_FILE))?;
    validate_schema_version(&extension.schema_version)?;
    validate_pnlx_manifest_values(&extension)?;
    // Canonicalize a two-part upstream version (e.g. pcre2 `10.43`) so the path it
    // installs to, the lockfile, and the `=<version>` pin in `pnl.json` are all valid
    // three-part semver.
    extension.version = crate::model::version::to_canonical_semver(&extension.version);

    // Refuse early (before running any install scripts) when the package does
    // not declare support for this platform — e.g. a library that is not
    // packaged for Alpine/musl.
    let current = crate::model::platform::current_platform_requirement();
    if !crate::model::platform::platform_supported(&extension.platforms, &current) {
        let supported = extension
            .platforms
            .iter()
            .map(crate::model::platform::describe_platform)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} {} does not support this platform ({}); supported platforms: {}",
            extension.name,
            extension.version,
            crate::model::platform::describe_platform(&current),
            supported
        );
    }

    // Fail fast with an actionable, platform-specific message before installing
    // dependencies and resolving native-library headers when libclang — the one hard
    // requirement for generating this package's bindings — is missing. Packages whose
    // requirements all use the curated verbatim-header path need no parse and skip it.
    if manifest_requires_libclang(&extension) {
        crate::native::header_adapter::ensure_libclang_available()?;
    }

    // Enforce an `@<version>` pin or a lockfile restore against the resolved version.
    // Both sides are canonicalized so a two-part pin (`@10.43`) matches `10.43.0`.
    if let Some(expected) = expected_version
        && extension.version != crate::model::version::to_canonical_semver(expected)
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
    verify_locked_integrity(
        root,
        &extension,
        &content_hash,
        options.force,
        &options.interaction,
    )?;
    crate::native::install_script::verify_install_scripts(
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
    let installed_extension_root = install_extension_files(
        &packages_root,
        extension_root,
        &extension.name,
        &extension.version,
    )?;

    // Dependency packages install nested under this package's own directory.
    let previous_packages_root = state
        .packages_root
        .replace(installed_extension_root.join("packages"));
    install_extension_dependencies(root, manifest, &extension, options, state)?;
    state.packages_root = previous_packages_root;

    if !is_dependency {
        set_manifest_extension_requirement(manifest, &extension.name, &extension.version);
        write_json(
            &root.join(crate::model::config::PNL_MANIFEST_FILE),
            manifest,
        )?;
    }

    // Offer to install the package's native dependencies (e.g. `brew install …`)
    // before we try to resolve them from disk.
    if let Some(script) = &extension.setup.build_script {
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
        &extension.compile_options.definitions,
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
    let mut dependency_libraries =
        resolve_dependency_libraries(root, manifest, &dependency_arch_entries)?;

    // Map a (recursive) dependency package's C functions to its entity class, so a
    // function-like macro that calls one resolves to that class instead of becoming
    // a thrower. The dependencies were installed (and locked) above.
    let dependency_functions = collect_dependency_functions(
        &installed_extension_root.join("packages"),
        &dependency_package_names,
    );
    // `compile_options.static_inline` (consumer-side): build a trampoline shim so
    // `static inline` functions become bound methods. Discover the C compiler once;
    // `shim::build` errors per package if one is actually needed but missing.
    let compile_static_inline = manifest.compile_options.static_inline;
    let shim_compiler = if compile_static_inline {
        crate::native::cc::find_c_compiler()
    } else {
        None
    };
    for (key, requirement) in &extension.native_libraries {
        let mut native = resolve_native_library(root, manifest, key, requirement)?;
        // Co-load libraries discovered from a GNU ld linker script (e.g. ncurses's
        // `INPUT(libncurses.so.6 -ltinfo)` → libtinfo) join the declared dependency
        // co-loads: their exports are unioned for the filter and they are loaded at
        // runtime, so a symbol split into a sibling `.so` (curses_version) resolves.
        dependency_libraries.append(&mut native.co_load);
        // Stamp the first-install time, preserving it across reinstalls so the
        // timestamp reflects when the library first entered the workspace.
        native.installed_at = pathmap
            .native_libraries
            .get(key)
            .and_then(|previous| previous.installed_at.clone())
            .or_else(|| Some(now()));
        crate::app::ui::success(&format!(
            "resolved {key} {} {}",
            native.version,
            crate::app::ui::dim(&native.resolved_name)
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
        // Read exports from the `.tbd` stub when one backs the library (macOS SDK):
        // `native.path` is the dylib the runtime loads, `export_source` the stub the
        // filter reads.
        let export_path = native
            .export_source
            .clone()
            .unwrap_or_else(|| native_path.clone());
        let exported = exported_symbols_union(&export_path, &dependency_libraries);
        pathmap.native_libraries.insert(key.clone(), native);
        pathmap.headers.insert(key.clone(), header);
        // The shim build uses the same headers, `-I` dirs (the libclang-derived set
        // plus pkg-config's), and `-D` definitions as the parse, links the primary
        // library, and writes into the package's `shim/` dir.
        let shim_request = compile_static_inline.then(|| {
            let mut include_dirs =
                crate::native::header_adapter::include_search_dirs(&generation_headers);
            for dir in &header_include_dirs {
                if !include_dirs.contains(dir) {
                    include_dirs.push(dir.clone());
                }
            }
            crate::native::shim::ShimRequest {
                compiler: shim_compiler.clone(),
                out_dir: installed_extension_root.join("shim"),
                stem: sanitize_shim_stem(key),
                headers: generation_headers.clone(),
                include_dirs,
                definitions: resolved_definitions.clone(),
                primary_library: native_path.clone(),
                package: extension.name.clone(),
            }
        });
        let shim_library = generate_installed_package_artifacts(
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
            shim_request.as_ref(),
        )?;
        // Co-load the shim alongside the package's own library so its `pnl_si_*`
        // exports resolve; recorded in `LIBRARIES` and the lock with the others.
        if let Some(shim_library) = shim_library {
            crate::app::ui::success(&format!(
                "built static-inline shim {}",
                crate::app::ui::dim(&shim_library.display().to_string())
            ));
            // Store an absolute path, like the other co-load libraries, so the
            // runtime resolves it regardless of the working directory.
            let shim_path = std::fs::canonicalize(&shim_library).unwrap_or(shim_library);
            dependency_libraries.insert(
                format!("{key}-static-inline-shim"),
                shim_path.to_string_lossy().into_owned(),
            );
        }
        if let Some(fqcn) = entity_class_fqn(&extension) {
            // Bake the resolved native library path + hash + co-load library paths
            // into the entity constants.
            let class_name = fqcn.rsplit('\\').next().unwrap_or(&fqcn);
            let library_paths: Vec<String> = dependency_libraries.values().cloned().collect();
            crate::codegen::stamp_entity_native_library(
                &installed_extension_root.join(crate::model::config::GENERATED_DIR),
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
                native_libraries: locked_requires,
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
    crate::app::ui::success(&format!("installed extension {}", extension.name));

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
            Ok(body) => crate::app::ui::example_block(&label, body.trim_end()),
            Err(_) => crate::app::ui::warn(&format!(
                "example file {example} is missing from the {} package",
                extension.name
            )),
        }
    }

    Ok(())
}

fn set_manifest_extension_requirement(manifest: &mut PnlManifest, package: &str, version: &str) {
    manifest.extensions.insert(
        package.to_owned(),
        ExtensionRequirement {
            version: format!("={version}"),
            required: true,
        },
    );
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
            // A bare-name dependency may pin a version (`vendor/lib@1.2.3`); that pin
            // is the exact version constraint this dependent places on the shared
            // library. With no pin, any installed version is acceptable.
            let (name, pin) = split_version_pin(package);
            let constraint = pin.map(|version| format!("={version}"));
            let installed = installed_dependency_version(root, name)?;
            match resolve_dependency_action(
                installed.as_deref(),
                constraint.as_deref(),
                name,
                &extension.name,
            )? {
                DependencyAction::Satisfied => {
                    crate::app::ui::info(&format!("dependency {name} already installed"));
                }
                DependencyAction::Install => {
                    crate::app::ui::step(&format!("installing dependency {package}"));
                    install_target(root, manifest, package, options, state)?;
                }
            }
        }
        truncate_manifest_repositories(manifest, added);
    }
    let _ = state.stack.pop();
    Ok(())
}

/// What to do about a declared dependency given what (if anything) is already in
/// the lockfile.
#[derive(Debug, PartialEq, Eq)]
enum DependencyAction {
    /// An installed version already satisfies the constraint; nothing to do.
    Satisfied,
    /// Not installed; install it.
    Install,
}

/// Decide a dependency's fate, detecting version conflicts: when a version is
/// already installed but does **not** satisfy the dependent's constraint, that is
/// an unsatisfiable requirement across the graph, reported instead of silently
/// using the wrong version (the previous behaviour, which deduped on any version).
fn resolve_dependency_action(
    installed: Option<&str>,
    constraint: Option<&str>,
    name: &str,
    dependent: &str,
) -> Result<DependencyAction> {
    let Some(version) = installed else {
        return Ok(DependencyAction::Install);
    };
    // No pin means any installed version is acceptable.
    let Some(constraint) = constraint else {
        return Ok(DependencyAction::Satisfied);
    };
    if installed_version_satisfies(version, constraint)? {
        Ok(DependencyAction::Satisfied)
    } else {
        bail!(
            "version conflict for {name}: {version} is already installed, but {dependent} requires {constraint}.\n  \
             Reconcile the version constraints, or remove the conflicting extension and reinstall."
        )
    }
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

/// The version of a dependency recorded in the lockfile, if it is installed.
fn installed_dependency_version(root: &Path, package: &str) -> Result<Option<String>> {
    let lock = read_lock_for_current_platform(root)?;
    Ok(lock
        .extensions
        .get(package)
        .map(|installed| installed.version.clone()))
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
    entries: &[&crate::model::manifest::DependencyEntry],
) -> Result<BTreeMap<String, String>> {
    let mut resolved = BTreeMap::new();
    for entry in entries {
        if entry.library_names.is_empty() {
            continue;
        }
        let requirement = crate::model::manifest::NativeRequirement {
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
    let mut union = crate::native::discovery::exported_symbols(primary_path)?;
    for path in dependency_libraries.values() {
        if let Some(symbols) = crate::native::discovery::exported_symbols(path) {
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
            read_json::<PnlxManifest>(&version_dir.join(crate::model::config::PNLX_MANIFEST_FILE))
        else {
            continue;
        };
        if let Some(fqcn) = entity_class_fqn(&manifest) {
            // The dependency's entity class, made absolute for a static `::` call.
            let fqcn = format!("\\{}", fqcn.trim_start_matches('\\'));
            let generated = version_dir.join(crate::model::config::GENERATED_DIR);
            if let Ok(entries) = std::fs::read_dir(&generated) {
                for path in entries.flatten().map(|entry| entry.path()) {
                    if !path
                        .file_name()
                        .and_then(|file| file.to_str())
                        .is_some_and(|file| file.ends_with(crate::model::config::FFI_FILE_SUFFIX))
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
        collect_dependency_functions_in(&version_dir.join("packages"), &nested_names, map, seen);
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
        .find(|path| {
            path.join(crate::model::config::PNLX_MANIFEST_FILE)
                .is_file()
        })
}

/// A filesystem-safe stem for a per-library shim file, derived from the library key.
fn sanitize_shim_stem(key: &str) -> String {
    key.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

/// Reject an install whose content digest differs from a previously locked
/// digest for the *same* version — the hallmark of tampered-with content. A new
/// version is treated as a legitimate update and is allowed through.
fn verify_locked_integrity(
    root: &Path,
    extension: &PnlxManifest,
    content_hash: &str,
    force: bool,
    interaction: &crate::app::interaction::Interaction,
) -> Result<()> {
    let lock = read_lock_for_current_platform(root)?;
    let Some(existing) = lock.extensions.get(&extension.name) else {
        return Ok(());
    };

    if existing.version == extension.version && existing.dist.sha256 != content_hash {
        if force {
            // `--force`: trust the resolved content and let the caller overwrite
            // the locked digest instead of aborting.
            crate::app::ui::warn(&format!(
                "{name}: content does not match the lockfile digest; overwriting it because --force was given\n  \
                 was sha256: {expected}\n  \
                 now sha256: {actual}",
                name = extension.name,
                expected = existing.dist.sha256,
                actual = content_hash,
            ));
            return Ok(());
        }
        // Interactively offer to proceed as if `--force`; default No, the safe
        // choice (a mismatch may be tampering). Non-interactive installs abort.
        if interaction.can_prompt() {
            crate::app::ui::warn(&format!(
                "{name}: content does not match the lockfile digest.\n  \
                 was sha256: {expected}\n  \
                 now sha256: {actual}",
                name = extension.name,
                expected = existing.dist.sha256,
                actual = content_hash,
            ));
            if interaction.confirm(
                &format!(
                    "Install {} anyway, overwriting the locked digest (--force)?",
                    extension.name
                ),
                false,
            )? {
                return Ok(());
            }
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

    crate::app::ui::success(&format!("verified {} integrity", extension.name));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::package::tree_sha256;
    use super::{
        DependencyAction, ExtensionSource, InstallSource, is_trusted_extension_source,
        path_from_file_url, resolve_dependency_action, resolve_install_source, split_version_pin,
    };

    #[test]
    fn dependency_not_installed_is_installed() {
        assert_eq!(
            resolve_dependency_action(None, None, "vendor/lib", "vendor/app").unwrap(),
            DependencyAction::Install,
        );
        assert_eq!(
            resolve_dependency_action(None, Some("=1.2.3"), "vendor/lib", "vendor/app").unwrap(),
            DependencyAction::Install,
        );
    }

    #[test]
    fn installed_dependency_satisfying_the_constraint_is_skipped() {
        // No pin: any installed version is acceptable.
        assert_eq!(
            resolve_dependency_action(Some("1.2.3"), None, "vendor/lib", "vendor/app").unwrap(),
            DependencyAction::Satisfied,
        );
        // Pin satisfied by the installed version.
        assert_eq!(
            resolve_dependency_action(Some("1.2.3"), Some("=1.2.3"), "vendor/lib", "vendor/app")
                .unwrap(),
            DependencyAction::Satisfied,
        );
    }

    #[test]
    fn installed_dependency_violating_the_constraint_is_a_conflict() {
        let conflict =
            resolve_dependency_action(Some("1.2.3"), Some("=2.0.0"), "vendor/lib", "vendor/app");
        let message = conflict.unwrap_err().to_string();
        assert!(message.contains("version conflict for vendor/lib"));
        assert!(message.contains("1.2.3"));
        assert!(message.contains("vendor/app"));
    }

    fn int_definition(
        default: Option<serde_json::Value>,
    ) -> Vec<crate::model::manifest::RequireDefinition> {
        vec![crate::model::manifest::RequireDefinition {
            name: "WIDTH".to_owned(),
            description: String::new(),
            definition_type: crate::model::manifest::DefinitionType::Int,
            default,
        }]
    }

    #[test]
    fn require_definitions_use_default_when_non_interactive() {
        let definitions = int_definition(Some(serde_json::json!(8)));
        let interaction = crate::app::interaction::Interaction::new(true, false);
        let resolved = super::definitions::resolve_require_definitions(
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
        let interaction = crate::app::interaction::Interaction::new(true, false);
        let resolved = super::definitions::resolve_require_definitions(
            "vendor/pkg",
            &definitions,
            &prior,
            &interaction,
        )
        .unwrap();
        assert_eq!(resolved[0].value, "16");
    }

    #[test]
    fn require_definitions_error_when_unresolvable_and_non_interactive() {
        let definitions = int_definition(None);
        let interaction = crate::app::interaction::Interaction::new(true, false);
        let result = super::definitions::resolve_require_definitions(
            "vendor/pkg",
            &definitions,
            &std::collections::BTreeMap::new(),
            &interaction,
        );
        assert!(
            result.is_err(),
            "expected an error for an unresolvable definition"
        );
    }

    #[test]
    fn library_stem_drops_lib_prefix_and_version_suffix() {
        assert_eq!(super::library_stem("libgslcblas.so.0"), "gslcblas");
        assert_eq!(super::library_stem("libbrotlicommon.so"), "brotlicommon");
        assert_eq!(super::library_stem("foo"), "foo");
    }

    #[test]
    fn dependency_entries_match_current_arch_and_wildcard() {
        use crate::model::manifest::{DependencyEntry, LibraryName};
        let arch = crate::model::platform::current_platform_requirement().arch;
        let entry = |name: &str| DependencyEntry {
            library_names: vec![LibraryName::Plain(name.to_owned())],
            ..Default::default()
        };
        let mut manifest = crate::model::manifest::PnlxManifest::default();
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
            .flat_map(|entry| {
                entry
                    .library_names
                    .iter()
                    .map(|name| name.name().to_owned())
            })
            .collect();
        assert!(names.contains(&"libfoo.so".to_owned()), "{names:?}");
        assert!(names.contains(&"libbar.so".to_owned()), "{names:?}");
        assert!(!names.contains(&"libnope.so".to_owned()), "{names:?}");
    }

    #[test]
    fn resolves_linux_installation_keys_from_os_release() {
        use super::native_deps::linux_installation_keys;
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
        assert!(is_bare_package_name("vendor/pkg"));
        assert!(!is_bare_package_name("./pkg"));
        assert!(!is_bare_package_name("packages/vendor/pkg"));
        assert!(!is_bare_package_name("vendor/"));
        assert!(!is_bare_package_name("https://example.com/pkg"));
        assert!(!is_bare_package_name("git@github.com:o/r.git"));
        assert!(!is_bare_package_name(""));
    }

    #[test]
    fn reinstall_updates_the_manifest_requirement() {
        let mut manifest = crate::model::manifest::PnlManifest::default();

        super::set_manifest_extension_requirement(&mut manifest, "vendor/pkg", "1.0.0");
        super::set_manifest_extension_requirement(&mut manifest, "vendor/pkg", "1.1.0");

        let requirement = manifest.extensions.get("vendor/pkg").unwrap();
        assert_eq!(requirement.version, "=1.1.0");
        assert!(requirement.required);
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
        use crate::model::manifest::{Dist, LockedExtension, PnlLock, PnlxManifest, Source};
        use crate::model::platform::current_platform;
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
                    kind: crate::model::manifest::RepositoryType::File,
                    url: "file:///pkg".to_owned(),
                    reference: "1.0.0".to_owned(),
                },
                dist: Dist {
                    url: "file:///pkg".to_owned(),
                    sha256: locked_hash.clone(),
                },
                classes: Vec::new(),
                dependencies: BTreeMap::new(),
                native_libraries: BTreeMap::new(),
                libraries: BTreeMap::new(),
                definitions: BTreeMap::new(),
            },
        );
        crate::util::io::write_json(&super::pnl_lock_path(dir.path()), &lock).unwrap();

        let mut extension = PnlxManifest {
            name: "vendor/example".to_owned(),
            version: "1.0.0".to_owned(),
            ..PnlxManifest::default()
        };

        // Same version + matching digest is fine.
        assert!(
            verify_locked_integrity(
                dir.path(),
                &extension,
                &locked_hash,
                false,
                &crate::app::interaction::Interaction::default()
            )
            .is_ok()
        );
        // Same version + different digest is a tamper and must be rejected.
        assert!(
            verify_locked_integrity(
                dir.path(),
                &extension,
                &"b".repeat(64),
                false,
                &crate::app::interaction::Interaction::default()
            )
            .is_err()
        );
        // ...unless --force is given, which trusts the resolved content.
        assert!(
            verify_locked_integrity(
                dir.path(),
                &extension,
                &"b".repeat(64),
                true,
                &crate::app::interaction::Interaction::default()
            )
            .is_ok()
        );
        // A new version is a legitimate update and is allowed through.
        extension.version = "2.0.0".to_owned();
        assert!(
            verify_locked_integrity(
                dir.path(),
                &extension,
                &"b".repeat(64),
                false,
                &crate::app::interaction::Interaction::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn tree_hash_ignores_generated_and_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::model::config::PNLX_MANIFEST_FILE),
            "{}",
        )
        .unwrap();
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
        std::fs::write(package.join(crate::model::config::PNLX_MANIFEST_FILE), "{}").unwrap();

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

        let repository = git2::Repository::init(temp.path()).unwrap();
        repository
            .remote("origin", "git@github.com:m3m0r7/pnl-packages.git")
            .unwrap();

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
