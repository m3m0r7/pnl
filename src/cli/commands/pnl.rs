use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::interaction::Interaction;
use crate::io::{read_json, read_or_default, write_json, write_json_if_missing};
use crate::manifest::{PnlLock, PnlManifest, PnlxPathmap, Repository, RepositoryType};
use crate::validate::{ensure_platform_matches, validate_pnl_workspace};

mod bridge;
mod find;
mod index;
mod install;
mod native;
mod package;

pub(crate) use bridge::build_installed_bridges;

use install::{InstallOptions, install};
use package::{installed_package_dir, pnl_lock_path, pnlx_pathmap_path, write_pnlx_autoload};

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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
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
    },
    Update {
        package: Option<String>,
    },
    Uninstall {
        package: String,
    },
    List {
        #[command(subcommand)]
        subject: Option<ListSubject>,
    },
    /// List packages available from the configured repositories (plus the
    /// built-in default), optionally filtered by a glob pattern (e.g. `lib*`).
    Find {
        pattern: Option<String>,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Validate,
    Version,
    SelfUpgrade {
        /// Directory where the pnl/pnlx symlinks are placed.
        #[arg(long, default_value = "/usr/local/bin")]
        bin_dir: PathBuf,
        /// Install root holding versions/<version> and the `current` link
        /// [default: $PNL_HOME, then $XDG_DATA_HOME/pnl, then ~/.local/share/pnl].
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Remove data pnl caches between runs.
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
    Repos,
    Extensions {
        /// Optional glob pattern (e.g. `lib*`) filtering installed extensions.
        pattern: Option<String>,
    },
    Native,
    /// `pnl list lib*` — a bare glob pattern is treated as an extensions filter.
    #[command(external_subcommand)]
    Pattern(Vec<String>),
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    Add(RepoAdd),
    Remove {
        url: String,
    },
    /// Generate a repository-index.json for a directory of packages so the
    /// repository can be browsed with `pnl find` without cloning.
    Index(RepoIndex),
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
        Command::Init => init_pnl(Path::new(".")),
        Command::Install {
            targets,
            alias_class,
            function_prefix,
        } => install(
            Path::new("."),
            &targets,
            &InstallOptions {
                alias_class,
                function_prefix,
                interaction,
            },
        ),
        Command::Update { package } => update(Path::new("."), package.as_deref()),
        Command::Uninstall { package } => uninstall(Path::new("."), &package, interaction),
        Command::List { subject } => list(Path::new("."), subject),
        Command::Find { pattern } => find::find(Path::new("."), pattern.as_deref()),
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

fn init_pnl(root: &Path) -> Result<()> {
    let manifest_path = root.join("pnl.json");
    write_json_if_missing(&manifest_path, &PnlManifest::default())?;
    crate::ui::success(&format!("initialized {}", manifest_path.display()));
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
    let manifest = read_json::<PnlManifest>(&root.join("pnl.json"))?;
    for repo in manifest.repositories {
        println!("{:?} {}", repo.kind, repo.url);
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
        // `pnl list lib*` finds `acme/libusb` as well as `libusb`.
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
    }
    write_json(&root.join("pnl.json"), &manifest)?;
    Ok(())
}
