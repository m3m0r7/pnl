use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::interaction::Interaction;
use crate::io::{read_json, read_or_default, write_json, write_json_if_missing};
use crate::manifest::{PnlLock, PnlManifest, PnlxPathmap, Repository, RepositoryType};
use crate::validate::{ensure_platform_matches, validate_pnl_workspace};

mod bridge;
mod index;
mod info;
mod install;
mod native;
mod package;
mod search;

pub(crate) use bridge::build_installed_bridges;

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
        /// Persist `features.use_functions = true` into pnl.json (exposes the
        /// generated global functions API).
        #[arg(long)]
        enable_use_functions: bool,
        /// Persist `features.allow_cdata = true` into pnl.json (also exposes raw
        /// `\FFI\CData` in generated signatures).
        #[arg(long)]
        enable_allow_cdata: bool,
        /// Persist `features.use_php_scalars_in_params = true` into pnl.json
        /// (generated methods accept a raw PHP scalar argument, not only a wrapper).
        #[arg(long)]
        enable_use_php_scalars_in_params: bool,
        /// Persist `features.use_php_scalars_in_return = true` into pnl.json
        /// (generated methods return PHP native int/float/string for scalars that fit).
        #[arg(long)]
        enable_use_php_scalars_in_return: bool,
        /// Reinstall even when the resolved content no longer matches the sha256
        /// recorded in the lockfile; the locked digest is overwritten.
        #[arg(long, short = 'f')]
        force: bool,
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
    crate::ui::set_debug(cli.debug);
    if cli.information {
        return crate::about::print_information(crate::about::Tool::Pnl);
    }
    if cli.license {
        return crate::about::print_license();
    }
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
        return Ok(());
    };

    // Surface a newer release at startup, except for the commands that already
    // talk about versions (self-upgrade) or caching (purge) themselves.
    if !matches!(command, Command::SelfUpgrade { .. } | Command::Purge { .. }) {
        crate::release::notify_if_update_available();
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
            enable_use_functions,
            enable_allow_cdata,
            enable_use_php_scalars_in_params,
            enable_use_php_scalars_in_return,
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
                enable_use_functions,
                enable_allow_cdata,
                enable_use_php_scalars_in_params,
                enable_use_php_scalars_in_return,
                force,
            },
        ),
        Command::Update { package } => update(Path::new("."), package.as_deref()),
        Command::Uninstall { package } => uninstall(Path::new("."), &package, interaction),
        Command::List { subject } => list(Path::new("."), subject),
        Command::Search { pattern } => search::search(Path::new("."), pattern.as_deref()),
        Command::Info { target } => info::info(Path::new("."), &target),
        Command::Repo { command } => repo(Path::new("."), command),
        Command::Validate => validate_pnl_workspace(Path::new(".")),
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::SelfUpgrade { bin_dir, home } => {
            crate::self_upgrade::self_upgrade(&bin_dir, home.as_deref())
        }
        Command::Purge { target } => match target {
            PurgeTarget::Cache => purge_cache(),
        },
    }
}

fn purge_cache() -> Result<()> {
    crate::ui::heading("pnl", "purge cache");
    let root = crate::cache::root();
    if crate::cache::purge()? {
        crate::ui::success(&format!("removed {}", root.display()));
    } else {
        crate::ui::info(&format!("no cache to remove at {}", root.display()));
    }
    Ok(())
}

fn init_pnl(root: &Path, interaction: Interaction) -> Result<()> {
    let manifest_path = root.join("pnl.json");
    write_json_if_missing(&manifest_path, &PnlManifest::default())?;

    // Scaffold the @pnlx workspace up front (autoload + SDK runtime + ide-helper),
    // and record the project manifest in the pathmap, so PHP can load the SDK even
    // before any extension is installed.
    write_pnlx_autoload(root)?;
    write_pathmap(root, &PnlxPathmap::empty_current())?;
    crate::ui::success(&format!("initialized {}", manifest_path.display()));

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

fn uninstall(root: &Path, package: &str, interaction: Interaction) -> Result<()> {
    let mut manifest = read_json::<PnlManifest>(&root.join("pnl.json"))?;
    if !manifest.extensions.contains_key(package) {
        bail!("{package} is not installed");
    }
    if !interaction.confirm(&format!("Remove extension {package}?"), true)? {
        crate::ui::warn("aborted");
        return Ok(());
    }
    manifest.extensions.remove(package);
    write_json(&root.join("pnl.json"), &manifest)?;

    let lock_path = pnl_lock_path(root);
    if lock_path.exists() {
        let mut lock = read_json::<PnlLock>(&lock_path)?;
        ensure_platform_matches(&lock.platform)?;
        lock.extensions.remove(package);
        lock.generated_at = crate::platform::now();
        write_json(&lock_path, &lock)?;
    }

    let install_dir = installed_package_dir(root, package);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    write_pnlx_autoload(root)?;
    crate::ui::summary(&format!("removed {package}"));
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
    let manifest = read_or_default::<PnlManifest>(&root.join("pnl.json"))?;
    for repo in install::resolved_repositories(&manifest) {
        let priority = repo.priority.unwrap_or(0);
        println!(
            "{:?} {} {}",
            repo.kind,
            repo.url,
            crate::ui::dim(&format!("(priority {priority})")),
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
            && !crate::glob::package_name_matches(pattern, &name)
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
    for (name, native) in pathmap.requires {
        println!("{} {} {}", name, native.version, native.path);
    }
    Ok(())
}

fn repo(root: &Path, command: RepoCommand) -> Result<()> {
    let mut manifest = read_or_default::<PnlManifest>(&root.join("pnl.json"))?;
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
            let signature = crate::repository_index::sign_index_file(
                &args.index,
                &args.key,
                args.output.as_deref(),
            )?;
            let public_key = crate::repository_index::public_key_from_secret(&args.key)?;
            crate::ui::success(&format!("signed {}", signature.display()));
            crate::ui::info(&format!("repository key: {public_key}"));
            return Ok(());
        }
    }
    write_json(&root.join("pnl.json"), &manifest)?;
    Ok(())
}
