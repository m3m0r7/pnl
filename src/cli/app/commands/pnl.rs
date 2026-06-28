use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::app::interaction::Interaction;
use crate::model::manifest::{PnlLock, PnlManifest, PnlxPathmap, Repository, RepositoryType};
use crate::model::validate::{ensure_platform_matches, validate_pnl_workspace};
use crate::util::io::{read_json, read_or_default, write_json, write_json_if_missing};

mod compose;
mod index;
mod info;
mod install;
mod package;
mod search;

use install::{InstallOptions, install};
use package::{
    installed_package_dir, pnl_lock_path, pnlx_pathmap_path, write_pathmap, write_pnlx_autoload,
};

#[derive(Debug, Parser)]
#[command(name = "pnl")]
#[command(about = "Manage PHP native-library extensions")]
#[command(version)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Do not ask interactive questions; accept the default answer for each prompt.
    #[arg(short = 'n', long, global = true)]
    no_interaction: bool,

    /// Answer "yes" to every confirmation (e.g. native-dependency installation).
    #[arg(short = 'y', long, global = true)]
    yes: bool,

    /// Show environment, version, and installed-extension information.
    #[arg(short = 'i', long)]
    information: bool,

    /// Show the pnl license and third-party component licenses.
    #[arg(short = 'l', long)]
    license: bool,

    /// Print verbose `debug:` diagnostics (resolution, paths, network) to stderr.
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a pnl.json manifest in the current directory.
    Init,
    /// Install one or more extensions and their native dependencies.
    Install {
        /// One or more sources (URL, path, or bare package name). With none,
        /// every extension is restored from the lockfile.
        targets: Vec<String>,
        /// Also define a `class_alias` so the extension can be referenced by this
        /// class name in addition to the one declared in pnlx.json.
        #[arg(long)]
        alias_class: Option<String>,
        /// Prefix added to every generated function and method name (replaces the
        /// unprefixed names).
        #[arg(long)]
        function_prefix: Option<String>,
        /// Continue even when install scripts are missing or fail their
        /// publish-time hash check.
        #[arg(long)]
        allow_unverified_install_scripts: bool,
        /// Explicit install-script hash to trust for this run. Can be repeated.
        #[arg(long)]
        allow_install_script_hash: Vec<String>,
        /// Persist `features.global_functions = true` into pnl.json (exposes the
        /// generated global functions API).
        #[arg(long)]
        enable_global_functions: bool,
        /// Persist `features.cdata_arguments = true` into pnl.json (also exposes raw
        /// `\FFI\CData` in generated signatures).
        #[arg(long)]
        enable_cdata_arguments: bool,
        /// Persist `features.scalar_returns = true` into pnl.json
        /// (generated methods return PHP native int/float/string for scalars that fit).
        #[arg(long)]
        enable_scalar_returns: bool,
        /// Persist `features.scalar_constants = true` into pnl.json
        /// (const.php uses PHP native int/float/string for losslessly representable
        /// values instead of `Pnlx\Types\*` wrappers).
        #[arg(long)]
        enable_scalar_constants: bool,
        /// Persist `compile_options.static_inline = true` into pnl.json: build a
        /// trampoline shim (needs a C compiler) so a library's `static inline`
        /// functions become bound methods instead of throwing stubs.
        #[arg(long)]
        enable_static_inline: bool,
        /// Reinstall even when the resolved content no longer matches the sha256
        /// recorded in the lockfile; the locked digest is overwritten.
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Compose installed extensions into one class exposing all their functions
    /// through a single shared FFI scope (a named counterpart to
    /// `Pnlx\Runtime::compose([...])`).
    Compose {
        /// Two or more installed package names (`vendor/package` or bare leaf).
        members: Vec<String>,
        /// The composed class FQN to generate, e.g. `Pnlx\Sdlx\Sdlx`.
        #[arg(long = "as")]
        as_class: String,
        /// Method-name prefix to resolve trait method collisions (reserved).
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Get or set a pnl.json configuration value (git-config style), e.g.
    /// `pnl config compile_options.static_inline true`. Omit the value to print
    /// the current one.
    Config {
        /// Dotted key, e.g. `compile_options.static_inline` or `features.global_functions`.
        key: String,
        /// New value (true/1/yes/on or false/0/no/off for booleans). Omit to print
        /// the current value.
        value: Option<String>,
        /// Reset the key to its default instead of setting a value.
        #[arg(long)]
        unset: bool,
    },
    /// Reinstall an extension (or all of them) from its recorded source.
    Update {
        /// Extension to update; omit to update every installed extension.
        package: Option<String>,
    },
    /// Remove an installed extension from the workspace.
    Uninstall {
        /// Extension name (e.g. `vendor/package`) to remove.
        package: String,
    },
    /// List installed extensions, repositories, or resolved native libraries.
    List {
        #[command(subcommand)]
        subject: Option<ListSubject>,
    },
    /// Search packages in the configured repositories (plus the built-in
    /// default), optionally filtered by a glob pattern (e.g. `lib*`).
    #[command(visible_alias = "find")]
    Search {
        /// Optional glob pattern (e.g. `lib*`) filtering package names.
        pattern: Option<String>,
    },
    /// Show a package's remote details: install commands, headers, and the
    /// native libraries it links — fetched from the repository even if it is
    /// already installed locally.
    Info {
        /// Package to describe (a bare name, `vendor/package`, URL,
        /// or path).
        target: String,
    },
    /// Manage package repositories recorded in pnl.json.
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    /// Validate pnl.json, the lockfile, and the pathmap against their schemas.
    Validate,
    /// Diagnose the environment: libclang, C compiler, PHP + FFI, and workspace.
    Doctor,
    /// Print the pnl version.
    Version,
    /// Download and install a newer pnl release (managed installs only).
    SelfUpgrade {
        /// Directory where the pnl/pnlx symlinks are placed.
        #[arg(long, default_value = "/usr/local/bin")]
        bin_dir: PathBuf,
        /// Install root holding versions/<version> and the `current` link
        /// [default: $PNL_HOME, then $XDG_DATA_HOME/pnl, then ~/.local/share/pnl].
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Remove pnl's on-disk caches between runs.
    Purge {
        #[command(subcommand)]
        target: PurgeTarget,
    },
}

#[derive(Debug, Subcommand)]
enum PurgeTarget {
    /// Remove pnl's on-disk caches (downloads and lookups) under
    /// `$XDG_CACHE_HOME/pnl`.
    Cache,
}

#[derive(Debug, Subcommand)]
enum ListSubject {
    /// List configured repositories plus the built-in default.
    #[command(visible_alias = "repo")]
    Repos,
    /// List installed extensions (the default), optionally glob-filtered.
    Extensions {
        /// Optional glob pattern (e.g. `lib*`) filtering installed extensions.
        pattern: Option<String>,
    },
    /// List the native libraries resolved into the pathmap.
    Native,
    /// `pnl list lib*` — a bare glob pattern is treated as an extensions filter.
    #[command(external_subcommand)]
    Pattern(Vec<String>),
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    /// Add a repository to pnl.json.
    Add(RepoAdd),
    /// Remove a repository from pnl.json by URL.
    Remove {
        /// Repository URL to remove.
        url: String,
    },
    /// Generate a repository-index.json for a directory of packages so the
    /// repository can be browsed with `pnl search` without cloning.
    Index(RepoIndex),
    /// Sign a repository-index.json with an Ed25519 secret key.
    Sign(RepoSign),
}

#[derive(Debug, Args)]
struct RepoIndex {
    /// Directory holding the packages to index.
    dir: PathBuf,
    /// Installable base URL each package directory is appended to (e.g.
    /// `https://github.com/m3m0r7/pnl-packages/tree/main/packages`).
    #[arg(long)]
    base_url: String,
    /// Output path [default: <dir>/repository-index.json].
    #[arg(long)]
    output: Option<PathBuf>,
    /// Git reference recorded for every version [default: the package version].
    #[arg(long)]
    reference: Option<String>,
}

#[derive(Debug, Args)]
struct RepoSign {
    /// repository-index.json to sign.
    index: PathBuf,
    /// Ed25519 secret key as `ed25519:<base64>` or 64 hex chars.
    #[arg(long)]
    key: String,
    /// Signature output [default: <index>.sig].
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RepoAdd {
    #[arg(value_enum)]
    kind: RepoKind,
    url: String,
    #[arg(long)]
    key: Option<String>,
    /// Resolution priority; higher is consulted first (defaults to 0).
    #[arg(long)]
    priority: Option<i64>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum RepoKind {
    Git,
    File,
    Https,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    crate::app::ui::set_debug(cli.debug);
    if cli.information {
        return crate::app::about::print_information(crate::app::about::Tool::Pnl);
    }
    if cli.license {
        return crate::app::about::print_license();
    }
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
        return Ok(());
    };

    // Surface a newer release at startup, except for the commands that already
    // talk about versions (self-upgrade) or caching (purge) themselves.
    if !matches!(command, Command::SelfUpgrade { .. } | Command::Purge { .. }) {
        crate::app::release::notify_if_update_available();
    }

    let interaction = Interaction::new(cli.no_interaction, cli.yes);
    match command {
        Command::Init => init_pnl(Path::new("."), interaction),
        Command::Install {
            targets,
            alias_class,
            function_prefix,
            allow_unverified_install_scripts,
            allow_install_script_hash,
            enable_global_functions,
            enable_cdata_arguments,
            enable_scalar_returns,
            enable_scalar_constants,
            enable_static_inline,
            force,
        } => install(
            Path::new("."),
            &targets,
            &InstallOptions {
                alias_class,
                function_prefix,
                interaction,
                allow_unverified_install_scripts,
                allowed_install_script_hashes: allow_install_script_hash,
                enable_global_functions,
                enable_cdata_arguments,
                enable_scalar_returns,
                enable_scalar_constants,
                enable_static_inline,
                force,
            },
        ),
        Command::Compose {
            members,
            as_class,
            prefix,
        } => compose::compose(Path::new("."), &members, &as_class, prefix.as_deref()),
        Command::Config { key, value, unset } => {
            config_command(Path::new("."), &key, value.as_deref(), unset, interaction)
        }
        Command::Update { package } => update(Path::new("."), package.as_deref()),
        Command::Uninstall { package } => uninstall(Path::new("."), &package, interaction),
        Command::List { subject } => list(Path::new("."), subject),
        Command::Search { pattern } => search::search(Path::new("."), pattern.as_deref()),
        Command::Info { target } => info::info(Path::new("."), &target),
        Command::Repo { command } => repo(Path::new("."), command),
        Command::Validate => validate_pnl_workspace(Path::new(".")),
        Command::Doctor => crate::app::doctor::run(Path::new(".")),
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::SelfUpgrade { bin_dir, home } => {
            crate::app::self_upgrade::self_upgrade(&bin_dir, home.as_deref())
        }
        Command::Purge { target } => match target {
            PurgeTarget::Cache => purge_cache(),
        },
    }
}

fn purge_cache() -> Result<()> {
    crate::app::ui::heading("pnl", "purge cache");
    let root = crate::sources::cache::root();
    if crate::sources::cache::purge()? {
        crate::app::ui::success(&format!("removed {}", root.display()));
    } else {
        crate::app::ui::info(&format!("no cache to remove at {}", root.display()));
    }
    Ok(())
}

fn init_pnl(root: &Path, interaction: Interaction) -> Result<()> {
    let manifest_path = root.join(crate::model::config::PNL_MANIFEST_FILE);
    write_json_if_missing(&manifest_path, &PnlManifest::default())?;

    // Scaffold the @pnlx workspace up front (autoload + SDK runtime + ide-helper),
    // and record the project manifest in the pathmap, so PHP can load the SDK even
    // before any extension is installed.
    write_pnlx_autoload(root)?;
    write_pathmap(root, &PnlxPathmap::empty_current())?;
    crate::app::ui::success(&format!("initialized {}", manifest_path.display()));

    install::offer_gitignore(root, &interaction)?;
    Ok(())
}

fn update(root: &Path, package: Option<&str>) -> Result<()> {
    let lock = read_json::<PnlLock>(&pnl_lock_path(root))?;
    ensure_platform_matches(&lock.platform)?;
    match package {
        Some(package) => {
            let entry = lock
                .extensions
                .get(package)
                .with_context(|| format!("{package} is not installed"))?;
            install(
                root,
                std::slice::from_ref(&entry.source.url),
                &InstallOptions::default(),
            )
        }
        None => {
            for entry in lock.extensions.values() {
                install(
                    root,
                    std::slice::from_ref(&entry.source.url),
                    &InstallOptions::default(),
                )?;
            }
            Ok(())
        }
    }
}

/// The pnl.json keys `pnl config` can read and write.
const KNOWN_CONFIG_KEYS: &[&str] = &[
    "compile_options.static_inline",
    "features.global_functions",
    "features.cdata_arguments",
    "features.scalar_params",
    "features.scalar_returns",
    "features.scalar_constants",
    "output_dir",
];

/// `pnl config <key> [value]` — git-config-style get/set/unset of a pnl.json value,
/// validated against the typed manifest. After a change that affects generated
/// output, offer to reinstall so it takes effect.
fn config_command(
    root: &Path,
    key: &str,
    value: Option<&str>,
    unset: bool,
    interaction: Interaction,
) -> Result<()> {
    let manifest_path = root.join(crate::model::config::PNL_MANIFEST_FILE);
    let mut manifest = read_or_default::<PnlManifest>(&manifest_path)?;

    // No value and no --unset: print the current value.
    if value.is_none() && !unset {
        println!("{}", config_get(&manifest, key)?);
        return Ok(());
    }

    config_apply(&mut manifest, key, value, unset)?;
    write_json(&manifest_path, &manifest)?;
    crate::app::ui::success(&format!(
        "{} {key}{}",
        if unset { "unset" } else { "set" },
        value
            .filter(|_| !unset)
            .map(|v| format!(" = {v}"))
            .unwrap_or_default(),
    ));

    // Every config key affects generated output, which is only (re)built on install.
    offer_config_reinstall(root, interaction)
}

/// The current value of a config key as a display string.
fn config_get(manifest: &PnlManifest, key: &str) -> Result<String> {
    Ok(match key {
        "compile_options.static_inline" => manifest.compile_options.static_inline.to_string(),
        "features.global_functions" => manifest.features.global_functions.to_string(),
        "features.cdata_arguments" => manifest.features.cdata_arguments.to_string(),
        "features.scalar_params" => manifest.features.scalar_params.to_string(),
        "features.scalar_returns" => manifest.features.scalar_returns.to_string(),
        "features.scalar_constants" => manifest.features.scalar_constants.to_string(),
        "output_dir" => manifest.output_dir.clone(),
        other => bail!(
            "unknown config key `{other}`. Known keys: {}",
            KNOWN_CONFIG_KEYS.join(", ")
        ),
    })
}

/// Set (or `--unset` to its default) a config key on the manifest.
fn config_apply(
    manifest: &mut PnlManifest,
    key: &str,
    value: Option<&str>,
    unset: bool,
) -> Result<()> {
    match key {
        "compile_options.static_inline" => {
            manifest.compile_options.static_inline = config_bool(value, unset, false)?;
        }
        "features.global_functions" => {
            manifest.features.global_functions = config_bool(value, unset, false)?;
        }
        "features.cdata_arguments" => {
            manifest.features.cdata_arguments = config_bool(value, unset, false)?;
        }
        "features.scalar_params" => {
            manifest.features.scalar_params = config_bool(value, unset, true)?;
        }
        "features.scalar_returns" => {
            manifest.features.scalar_returns = config_bool(value, unset, false)?;
        }
        "features.scalar_constants" => {
            manifest.features.scalar_constants = config_bool(value, unset, false)?;
        }
        "output_dir" => {
            manifest.output_dir = if unset {
                crate::model::config::DEFAULT_OUTPUT_DIR.to_owned()
            } else {
                value.context("a value is required")?.to_owned()
            };
        }
        other => bail!(
            "unknown config key `{other}`. Known keys: {}",
            KNOWN_CONFIG_KEYS.join(", ")
        ),
    }
    Ok(())
}

/// Parse a boolean config value (`--unset` yields `default`).
fn config_bool(value: Option<&str>, unset: bool, default: bool) -> Result<bool> {
    if unset {
        return Ok(default);
    }
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => bail!("`{other}` is not a boolean (use true/false, 1/0, yes/no, on/off)"),
    }
}

/// After a config change, offer to reinstall so generated output reflects it. A
/// non-interactive run just warns the change is pending; with no installed
/// extensions there is nothing to rebuild.
fn offer_config_reinstall(root: &Path, interaction: Interaction) -> Result<()> {
    let lock_path = pnl_lock_path(root);
    let has_installed = lock_path.exists()
        && read_json::<PnlLock>(&lock_path)
            .map(|lock| !lock.extensions.is_empty())
            .unwrap_or(false);
    if !has_installed {
        return Ok(());
    }
    if !interaction.can_prompt() {
        crate::app::ui::warn("the change affects generated output; run `pnl install` to apply it");
        return Ok(());
    }
    if interaction.confirm("Reinstall now to apply the change?", true)? {
        install(
            root,
            &[],
            &InstallOptions {
                interaction,
                ..InstallOptions::default()
            },
        )
    } else {
        crate::app::ui::warn(
            "not reinstalled; existing extensions keep their current build until you run `pnl install`",
        );
        Ok(())
    }
}

fn uninstall(root: &Path, package: &str, interaction: Interaction) -> Result<()> {
    let mut manifest =
        read_json::<PnlManifest>(&root.join(crate::model::config::PNL_MANIFEST_FILE))?;
    if !manifest.extensions.contains_key(package) {
        bail!("{package} is not installed");
    }
    if !interaction.confirm(&format!("Remove extension {package}?"), true)? {
        crate::app::ui::warn("aborted");
        return Ok(());
    }
    manifest.extensions.remove(package);
    write_json(
        &root.join(crate::model::config::PNL_MANIFEST_FILE),
        &manifest,
    )?;

    let lock_path = pnl_lock_path(root);
    if lock_path.exists() {
        let mut lock = read_json::<PnlLock>(&lock_path)?;
        ensure_platform_matches(&lock.platform)?;
        lock.extensions.remove(package);
        lock.generated_at = crate::model::platform::now();
        write_json(&lock_path, &lock)?;
    }

    let install_dir = installed_package_dir(root, package);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    write_pnlx_autoload(root)?;
    crate::app::ui::summary(&format!("removed {package}"));
    Ok(())
}

fn list(root: &Path, subject: Option<ListSubject>) -> Result<()> {
    match subject {
        None | Some(ListSubject::Extensions { pattern: None }) => list_extensions(root, None),
        Some(ListSubject::Extensions { pattern }) => list_extensions(root, pattern.as_deref()),
        Some(ListSubject::Repos) => list_repositories(root),
        Some(ListSubject::Native) => list_native_libraries(root),
        // A bare `pnl list <glob>` arrives here; the first token is the pattern.
        Some(ListSubject::Pattern(args)) => list_extensions(root, args.first().map(String::as_str)),
    }
}

fn list_repositories(root: &Path) -> Result<()> {
    // Show the same set bare-name resolution consults: the configured
    // repositories (highest priority first) plus the built-in default
    // (`pnl-packages`) appended as the lowest-priority fallback.
    let manifest =
        read_or_default::<PnlManifest>(&root.join(crate::model::config::PNL_MANIFEST_FILE))?;
    for repo in install::resolved_repositories(&manifest) {
        let priority = repo.priority.unwrap_or(0);
        println!(
            "{:?} {} {}",
            repo.kind,
            repo.url,
            crate::app::ui::dim(&format!("(priority {priority})")),
        );
    }
    Ok(())
}

fn list_extensions(root: &Path, pattern: Option<&str>) -> Result<()> {
    let lock_path = pnl_lock_path(root);
    if !lock_path.exists() {
        return Ok(());
    }
    let lock = read_json::<PnlLock>(&lock_path)?;
    ensure_platform_matches(&lock.platform)?;
    for (name, ext) in lock.extensions {
        // Match against both the full `vendor/extension` name and its leaf, so
        // `pnl list gfx*` finds `acme/gfx` as well as `gfx`.
        if let Some(pattern) = pattern
            && !crate::util::glob::package_name_matches(pattern, &name)
        {
            continue;
        }
        println!("{} {} {}", name, ext.version, ext.source.reference);
    }
    Ok(())
}

fn list_native_libraries(root: &Path) -> Result<()> {
    let pathmap_path = pnlx_pathmap_path(root);
    if !pathmap_path.exists() {
        return Ok(());
    }
    let pathmap = read_json::<PnlxPathmap>(&pathmap_path)?;
    ensure_platform_matches(&pathmap.platform)?;
    for (name, native) in pathmap.native_libraries {
        println!("{} {} {}", name, native.version, native.path);
    }
    Ok(())
}

fn repo(root: &Path, command: RepoCommand) -> Result<()> {
    let mut manifest =
        read_or_default::<PnlManifest>(&root.join(crate::model::config::PNL_MANIFEST_FILE))?;
    match command {
        RepoCommand::Add(add) => {
            let repo = Repository {
                kind: match add.kind {
                    RepoKind::Git => RepositoryType::Git,
                    RepoKind::File => RepositoryType::File,
                    RepoKind::Https => RepositoryType::Https,
                },
                url: add.url,
                key: add.key,
                priority: add.priority,
            };
            if !manifest
                .repositories
                .iter()
                .any(|item| item.url == repo.url)
            {
                manifest.repositories.push(repo);
            }
        }
        RepoCommand::Remove { url } => {
            manifest.repositories.retain(|repo| repo.url != url);
        }
        RepoCommand::Index(args) => {
            return index::generate_index(
                &args.dir,
                &args.base_url,
                args.output.as_deref(),
                args.reference.as_deref(),
            );
        }
        RepoCommand::Sign(args) => {
            let signature = crate::model::repository_index::sign_index_file(
                &args.index,
                &args.key,
                args.output.as_deref(),
            )?;
            let public_key = crate::model::repository_index::public_key_from_secret(&args.key)?;
            crate::app::ui::success(&format!("signed {}", signature.display()));
            crate::app::ui::info(&format!("repository key: {public_key}"));
            return Ok(());
        }
    }
    write_json(
        &root.join(crate::model::config::PNL_MANIFEST_FILE),
        &manifest,
    )?;
    Ok(())
}
