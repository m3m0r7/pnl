use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::commands::pnl::build_installed_bridges;
use crate::generate::{
    PhpPackageTemplateOptions, generate_aliases_php, generate_bridge_ffi_php, generate_bridge_rs,
    generate_const_php, generate_context_php, generate_entity_php, generate_exception_php,
    generate_functions_php, generate_index_php, generate_manifest_php, generate_types_php,
    parse_function_signatures,
};
use crate::header_adapter::{HeaderAdapterOptions, cdef_from_header};
use crate::interaction::Interaction;
use crate::io::{read_json, write_json, write_json_if_missing};
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
#[command(arg_required_else_help = true)]
struct Cli {
    /// Do not ask interactive questions; accept the default answer for each prompt.
    #[arg(short = 'n', long, global = true)]
    no_interaction: bool,

    /// Show environment, version, and installed-extension information.
    #[arg(short = 'i', long)]
    information: bool,

    /// Show the pnlx license and third-party component licenses.
    #[arg(short = 'l', long)]
    license: bool,

    #[command(subcommand)]
    command: Option<Command>,
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
    /// Stamp publish-time metadata into pnlx.json.
    Publish,
    Version,
    Package,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.information {
        return crate::about::print_information(crate::about::Tool::Pnlx);
    }
    if cli.license {
        return crate::about::print_license();
    }
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
        return Ok(());
    };

    crate::release::notify_if_update_available();

    // pnlx has no interactive prompts yet, but it accepts --no-interaction so
    // scripts can pass the same flags to both binaries.
    let _interaction = Interaction::new(cli.no_interaction, false);
    match command {
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
        Command::Publish => publish_pnlx(Path::new(".")),
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

    generate_all(&GenerateArtifacts {
        generated_dir: &root.join("src/generated"),
        artifact_stem: &artifact_stem,
        namespace: &namespace,
        class_name: &class_name,
        library_key: &library_key,
        symbol_prefix: symbol_prefix_for_library(&manifest, &library_key),
        alias_class: None,
        function_prefix: "",
        native_library_name: &manifest.name,
        native_library_version: &manifest.version,
        headers: &headers,
    })
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

    generate_all(&GenerateArtifacts {
        generated_dir: &root.join("src/generated"),
        artifact_stem: &artifact_stem,
        namespace: &namespace,
        class_name: &class_name,
        library_key,
        symbol_prefix: symbol_prefix_for_library(manifest, library_key),
        alias_class,
        function_prefix: function_prefix.unwrap_or(""),
        native_library_name: &manifest.name,
        native_library_version: &manifest.version,
        headers,
    })
}

/// Everything `generate_all` needs to emit a package's generated artifacts.
struct GenerateArtifacts<'a> {
    /// `<package>/src/generated` directory the artifacts are written into.
    generated_dir: &'a Path,
    /// File stem for the per-library `*.ffi.php` / `*.bridge.rs` artifacts.
    artifact_stem: &'a str,
    namespace: &'a str,
    class_name: &'a str,
    library_key: &'a str,
    symbol_prefix: Option<String>,
    alias_class: Option<&'a str>,
    function_prefix: &'a str,
    /// Native library package name/version, for the entity's `#[NativeLibrary*]`
    /// attributes.
    native_library_name: &'a str,
    native_library_version: &'a str,
    headers: &'a [PathBuf],
}

fn generate_all(args: &GenerateArtifacts<'_>) -> Result<()> {
    let generated_dir = args.generated_dir;
    let class_name = args.class_name;
    let out = generated_dir.join(format!("{}.ffi.php", args.artifact_stem));
    let (cdef, constants) = if args.headers.is_empty() {
        (read_existing_ffi_cdef(&out)?, Vec::new())
    } else {
        let artifacts = cdef_from_header(
            &read_headers(args.headers)?,
            &HeaderAdapterOptions {
                symbol_prefix: args
                    .symbol_prefix
                    .clone()
                    .unwrap_or_else(|| symbol_prefix_from_library_key(args.library_key)),
            },
        )?;
        (artifacts.cdef, artifacts.constants)
    };
    let signatures = parse_function_signatures(&cdef);
    generate_bridge_ffi_php(&signatures, &out)?;
    let ffi_file = out
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("extension.ffi.php");
    let template_options = PhpPackageTemplateOptions {
        namespace: args.namespace,
        class_name,
        library_key: args.library_key,
        ffi_file,
        signatures: &signatures,
        alias_class: args.alias_class,
        function_prefix: args.function_prefix,
        native_library_name: args.native_library_name,
        native_library_version: args.native_library_version,
    };
    // Metadata, the CData wrapper, and the per-extension exception are shared by
    // every entity variant.
    generate_manifest_php(
        &generated_dir.join(format!("{class_name}Manifest.php")),
        &template_options,
    )?;
    generate_context_php(
        &generated_dir.join(format!("{class_name}Context.php")),
        &template_options,
    )?;
    generate_exception_php(
        &generated_dir.join(format!("{class_name}Exception.php")),
        &template_options,
    )?;
    generate_types_php(&generated_dir.join("types"), &template_options)?;
    // Four entity variants on two axes, selected at runtime by `index.php`:
    // `allow_cdata` (the `cdata/` subdir, params also accept raw `\FFI\CData`) and
    // `use_php_scalars_in_return` (the `scalar/` subdir, methods return native
    // scalars). `use_php_scalars_in_params` is enforced at runtime, not by variant.
    for allow_cdata in [false, true] {
        for scalars_in_return in [false, true] {
            let mut dir = generated_dir.to_path_buf();
            if allow_cdata {
                dir.push("cdata");
            }
            if scalars_in_return {
                dir.push("scalar");
            }
            generate_entity_php(
                &dir.join(format!("{class_name}.php")),
                &template_options,
                allow_cdata,
                scalars_in_return,
            )?;
        }
    }
    generate_const_php(
        &generated_dir.join("const.php"),
        &template_options,
        &constants,
    )?;
    generate_index_php(&generated_dir.join("index.php"), &template_options)?;
    generate_functions_php(&generated_dir.join("functions.php"), &template_options)?;
    generate_aliases_php(&generated_dir.join("function.aliases.php"), &signatures)?;
    generate_bridge_rs(
        &generated_dir.join(format!("{}.bridge.rs", args.artifact_stem)),
        &template_options,
        &signatures,
    )?;
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

fn publish_pnlx(root: &Path) -> Result<()> {
    let manifest_path = root.join("pnlx.json");
    let mut manifest: PnlxManifest = read_json(&manifest_path)?;
    validate_pnlx_manifest_values(&manifest)?;

    match crate::install_script::install_script_hash(root, &manifest)? {
        Some(hash) => {
            manifest.install_script_hash = Some(hash.clone());
            write_json(&manifest_path, &manifest)?;
            crate::ui::success(&format!("stamped install_script_hash {hash}"));
        }
        None => {
            manifest.install_script_hash = None;
            write_json(&manifest_path, &manifest)?;
            crate::ui::info("no install scripts declared; cleared install_script_hash");
        }
    }

    Ok(())
}
