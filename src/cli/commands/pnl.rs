use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::interaction::Interaction;
use crate::io::{read_json, read_or_default, write_json, write_json_if_missing};
use crate::manifest::{PnlLock, PnlManifest, PnlxPathmap, Repository, RepositoryType};
use crate::validate::{ensure_platform_matches, validate_pnl_workspace};

mod bridge;
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
struct Cli {
    /// Do not ask interactive questions; accept the default answer for each prompt.
    #[arg(short = 'n', long, global = true)]
    no_interaction: bool,

    #[command(subcommand)]
    command: Command,
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
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Validate,
    Version,
    SelfUpgrade,
}

#[derive(Debug, Subcommand)]
enum ListSubject {
    Repos,
    Extensions,
    Native,
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    Add(RepoAdd),
    Remove { url: String },
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
    let interaction = Interaction::new(cli.no_interaction);
    match cli.command {
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
            },
        ),
        Command::Update { package } => update(Path::new("."), package.as_deref()),
        Command::Uninstall { package } => uninstall(Path::new("."), &package, interaction),
        Command::List { subject } => list(Path::new("."), subject),
        Command::Repo { command } => repo(Path::new("."), command),
        Command::Validate => validate_pnl_workspace(Path::new(".")),
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::SelfUpgrade => bail!("self-upgrade is not implemented yet"),
    }
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
    match subject.unwrap_or(ListSubject::Extensions) {
        ListSubject::Repos => list_repositories(root),
        ListSubject::Extensions => list_extensions(root),
        ListSubject::Native => list_native_libraries(root),
    }
}

fn list_repositories(root: &Path) -> Result<()> {
    let manifest = read_json::<PnlManifest>(&root.join("pnl.json"))?;
    for repo in manifest.repositories {
        println!("{:?} {}", repo.kind, repo.url);
    }
    Ok(())
}

fn list_extensions(root: &Path) -> Result<()> {
    let lock_path = pnl_lock_path(root);
    if !lock_path.exists() {
        return Ok(());
    }
    let lock = read_json::<PnlLock>(&lock_path)?;
    ensure_platform_matches(&lock.platform)?;
    for (name, ext) in lock.extensions {
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
    }
    write_json(&root.join("pnl.json"), &manifest)?;
    Ok(())
}
