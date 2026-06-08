use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::io::{read_json, read_or_default, write_json, write_json_if_missing};
use crate::manifest::{PnlLock, PnlManifest, PnlxPathmap, Repository, RepositoryType};
use crate::validate::{ensure_platform_matches, validate_pnl_workspace};

mod bridge;
mod install;
mod native;
mod package;

pub(crate) use bridge::build_installed_bridges;

use install::install;
use package::{installed_extension_dir, pnl_lock_path, pnlx_pathmap_path, write_pnlx_autoload};

#[derive(Debug, Parser)]
#[command(name = "pnl")]
#[command(about = "Manage PHP native-library extensions")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Install {
        target: Option<String>,
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
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum RepoKind {
    Git,
    File,
    Https,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => init_pnl(Path::new(".")),
        Command::Install { target } => install(Path::new("."), target.as_deref()),
        Command::Update { package } => update(Path::new("."), package.as_deref()),
        Command::Uninstall { package } => uninstall(Path::new("."), &package),
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
    println!("initialized {}", manifest_path.display());
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
            install(root, Some(&entry.source.url))
        }
        None => {
            for entry in lock.extensions.values() {
                install(root, Some(&entry.source.url))?;
            }
            Ok(())
        }
    }
}

fn uninstall(root: &Path, package: &str) -> Result<()> {
    let mut manifest = read_json::<PnlManifest>(&root.join("pnl.json"))?;
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

    let install_dir = installed_extension_dir(root, package);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    write_pnlx_autoload(root)?;
    println!("uninstalled {package}");
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
