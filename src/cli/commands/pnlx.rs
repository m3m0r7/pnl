use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::commands::pnl::build_installed_bridges;
use crate::generate::{
    PhpPackageTemplateOptions, generate_aliases_php, generate_bridge_ffi_php, generate_bridge_rs,
    generate_context_php, generate_entity_php, generate_functions_php, generate_index_php,
    parse_function_signatures,
};
use crate::header_adapter::{HeaderAdapterOptions, cdef_from_header};
use crate::interaction::Interaction;
use crate::io::{read_json, write_json_if_missing};
use crate::manifest::PnlxManifest;
use crate::validate::{validate_pnlx_manifest_values, validate_pnlx_workspace};

mod resolution;

use resolution::{
    resolve_headers, resolve_headers_from_pathmap, resolve_library_key, sanitize_artifact_stem,
    split_class, symbol_prefix_for_library, symbol_prefix_from_library_key,
};

#[derive(Debug, Parser)]
#[command(name = "pnlx")]
#[command(about = "Develop PHP native-library extensions")]
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
    Validate,
    Gen {
        target: String,
        #[arg(long)]
        library_key: Option<String>,
    },
    Build {
        packages: Vec<String>,
    },
    Version,
    Package,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    // pnlx has no interactive prompts yet, but it accepts --no-interaction so
    // scripts can pass the same flags to both binaries.
    let _interaction = Interaction::new(cli.no_interaction, false);
    match cli.command {
        Command::Init => init_pnlx(Path::new(".")),
        Command::Validate => validate_pnlx_workspace(Path::new(".")),
        Command::Gen {
            target,
            library_key,
        } => gen_pnlx(
            Path::new("."),
            GenOptions {
                target,
                library_key,
            },
        ),
        Command::Build { packages } => build_installed_bridges(Path::new("."), &packages),
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Package => bail!("pnlx package is not implemented yet"),
    }
}

#[derive(Debug)]
struct GenOptions {
    target: String,
    library_key: Option<String>,
}

fn gen_pnlx(root: &Path, options: GenOptions) -> Result<()> {
    let target = options.target.as_str();
    let manifest: PnlxManifest = read_json(&root.join("pnlx.json"))?;
    validate_pnlx_manifest_values(&manifest)?;
    let package_leaf = manifest.name.rsplit('/').next().unwrap_or(target);
    let artifact_stem = sanitize_artifact_stem(target);
    let (manifest_namespace, manifest_class) = split_class(&manifest.class)?;
    let namespace = manifest_namespace.replace("\\\\", "\\");
    let class_name = format!("{}{}", manifest.class_prefix, manifest_class);
    let library_key = options
        .library_key
        .clone()
        .map(Ok)
        .unwrap_or_else(|| resolve_library_key(target, package_leaf, &manifest))?;
    let headers =
        if let Some(headers) = resolve_headers_from_pathmap(root, &library_key, &manifest)? {
            headers
        } else {
            resolve_headers(target, &manifest)?
                .into_iter()
                .map(|header| root.join(&header.path))
                .collect()
        };

    let generated_dir = root.join("src/generated");
    let out = generated_dir.join(format!("{artifact_stem}.ffi.php"));
    let entity_out = generated_dir.join(format!("{class_name}.php"));
    let context_out = generated_dir.join(format!("{class_name}Context.php"));
    let index_out = generated_dir.join("index.php");
    let aliases_out = generated_dir.join("function.aliases.php");
    let bridge_out = generated_dir.join(format!("{artifact_stem}.bridge.rs"));

    generate_all(
        &headers,
        &out,
        &namespace,
        &class_name,
        &library_key,
        symbol_prefix_for_library(&manifest, &library_key).as_deref(),
        None,
        "",
        &entity_out,
        &context_out,
        &index_out,
        &aliases_out,
        &bridge_out,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_installed_package_artifacts(
    root: &Path,
    manifest: &PnlxManifest,
    target: &str,
    library_key: &str,
    headers: &[PathBuf],
    alias_class: Option<&str>,
    function_prefix: Option<&str>,
) -> Result<()> {
    let package_leaf = manifest.name.rsplit('/').next().unwrap_or(target);
    let artifact_stem = sanitize_artifact_stem(package_leaf);
    let (namespace, manifest_class) = split_class(&manifest.class)?;
    let class_name = format!("{}{}", manifest.class_prefix, manifest_class);
    let generated_dir = root.join("src/generated");

    generate_all(
        headers,
        &generated_dir.join(format!("{artifact_stem}.ffi.php")),
        &namespace,
        &class_name,
        library_key,
        symbol_prefix_for_library(manifest, library_key).as_deref(),
        alias_class,
        function_prefix.unwrap_or(""),
        &generated_dir.join(format!("{class_name}.php")),
        &generated_dir.join(format!("{class_name}Context.php")),
        &generated_dir.join("index.php"),
        &generated_dir.join("function.aliases.php"),
        &generated_dir.join(format!("{artifact_stem}.bridge.rs")),
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_all(
    headers: &[PathBuf],
    out: &Path,
    namespace: &str,
    class_name: &str,
    library_key: &str,
    symbol_prefix: Option<&str>,
    alias_class: Option<&str>,
    function_prefix: &str,
    entity_out: &Path,
    context_out: &Path,
    index_out: &Path,
    aliases_out: &Path,
    bridge_out: &Path,
) -> Result<()> {
    let cdef = if headers.is_empty() {
        read_existing_ffi_cdef(out)?
    } else {
        cdef_from_header(
            &read_headers(headers)?,
            &HeaderAdapterOptions {
                symbol_prefix: symbol_prefix
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| symbol_prefix_from_library_key(library_key)),
            },
        )?
    };
    let signatures = parse_function_signatures(&cdef);
    generate_bridge_ffi_php(&signatures, out)?;
    let ffi_file = out
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("extension.ffi.php");
    let template_options = PhpPackageTemplateOptions {
        namespace,
        class_name,
        library_key,
        ffi_file,
        signatures: &signatures,
        alias_class,
        function_prefix,
    };
    generate_entity_php(entity_out, &template_options)?;
    generate_context_php(context_out, &template_options)?;
    generate_index_php(index_out, &template_options)?;
    if let Some(generated_dir) = index_out.parent() {
        generate_functions_php(&generated_dir.join("functions.php"), &template_options)?;
    }
    generate_aliases_php(aliases_out, &signatures)?;
    generate_bridge_rs(bridge_out, &template_options, &signatures)?;
    Ok(())
}

fn read_existing_ffi_cdef(out: &Path) -> Result<String> {
    let content = std::fs::read_to_string(out).with_context(|| {
        format!(
            "no pnlx.json header was configured; failed to read existing {} as CDEF source",
            out.display()
        )
    })?;
    let start_marker = "return <<<'CDEF'\n";
    let start = content
        .find(start_marker)
        .with_context(|| format!("{} does not contain a generated CDEF block", out.display()))?
        + start_marker.len();
    let end = content[start..].find("\nCDEF;").with_context(|| {
        format!(
            "{} does not contain a generated CDEF terminator",
            out.display()
        )
    })? + start;

    Ok(content[start..end].to_owned())
}

fn read_headers(headers: &[PathBuf]) -> Result<String> {
    let mut content = String::new();
    for header in headers {
        content.push_str(
            &std::fs::read_to_string(header)
                .with_context(|| format!("failed to read {}", header.display()))?,
        );
        content.push('\n');
    }

    Ok(content)
}

fn init_pnlx(root: &Path) -> Result<()> {
    let manifest_path = root.join("pnlx.json");
    write_json_if_missing(&manifest_path, &PnlxManifest::default())?;
    crate::ui::success(&format!("initialized {}", manifest_path.display()));
    Ok(())
}
