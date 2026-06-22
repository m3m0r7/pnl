use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::generate::{
    PhpPackageTemplateOptions, generate_aliases_php, generate_component_php, generate_const_php,
    generate_context_php, generate_entity_php, generate_enums_php, generate_exception_php,
    generate_ffi_php_from_cdef, generate_functions_php, generate_index_php,
    generate_macro_functions_php, generate_manifest_php, generate_symbols_php, generate_types_php,
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
    let manifest: PnlxManifest = read_json(&root.join(crate::config::PNLX_MANIFEST_FILE))?;
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
    // Fail fast with an actionable, platform-specific message before resolving and
    // reading headers when libclang — the one hard requirement for this path — is
    // missing. Skipped for the curated verbatim-header libraries that need no parse.
    if library_requires_libclang(&manifest, &library_key) {
        crate::header_adapter::ensure_libclang_available()?;
    }
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
        generated_dir: &root.join(crate::config::GENERATED_DIR),
        artifact_stem: &artifact_stem,
        namespace: &namespace,
        class_name: &class_name,
        library_key: &library_key,
        symbol_prefix: symbol_prefix_for_library(&manifest, &library_key),
        alias_class: None,
        function_prefix: "",
        native_library_name: &manifest.name,
        native_library_version: &manifest.version,
        description: &manifest.description,
        headers: &headers,
        extra_include_dirs: &[],
        dependency_functions: &std::collections::BTreeMap::new(),
        exported_symbols: None,
        // Local `pnlx gen` resolves config-gated definitions from their declared
        // defaults only (no prompting, no lockfile); install does the interactive
        // resolution.
        definitions: &resolve_definition_defaults(&manifest.require_definitions),
    })
}

/// Resolve `require_definitions` to their declared defaults (no prompt, no lock),
/// for the local `pnlx gen` path. Entries without a default are skipped, so a
/// header that truly needs a value still fails to parse — which is the right signal
/// for a package author running `pnlx gen` without supplying one.
fn resolve_definition_defaults(
    definitions: &[crate::manifest::RequireDefinition],
) -> Vec<crate::manifest::ResolvedDefinition> {
    definitions
        .iter()
        .filter_map(|definition| {
            let value = match definition.default.as_ref()? {
                serde_json::Value::Bool(flag) => if *flag { "1" } else { "0" }.to_owned(),
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            Some(crate::manifest::ResolvedDefinition {
                name: definition.name.clone(),
                value,
                definition_type: definition.definition_type,
            })
        })
        .collect()
}

/// The symbol prefix the generator will actually use for `library_key`: the
/// requirement's declared prefix, else one derived from the key. An empty prefix is
/// the curated verbatim-header path (e.g. libc) that emits a cdef without libclang;
/// anything non-empty means libclang is required to read the C headers.
pub(crate) fn library_requires_libclang(manifest: &PnlxManifest, library_key: &str) -> bool {
    !symbol_prefix_for_library(manifest, library_key)
        .unwrap_or_else(|| symbol_prefix_from_library_key(library_key))
        .trim()
        .is_empty()
}

/// Whether generating any of this package's bindings will invoke libclang — used to
/// preflight the toolchain before downloading dependencies and native-library headers.
pub(crate) fn manifest_requires_libclang(manifest: &PnlxManifest) -> bool {
    manifest
        .requires
        .keys()
        .any(|key| library_requires_libclang(manifest, key))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_installed_package_artifacts(
    root: &Path,
    manifest: &PnlxManifest,
    target: &str,
    library_key: &str,
    headers: &[PathBuf],
    extra_include_dirs: &[PathBuf],
    alias_class: Option<&str>,
    function_prefix: Option<&str>,
    dependency_functions: &std::collections::BTreeMap<String, String>,
    exported_symbols: Option<&std::collections::BTreeSet<String>>,
    definitions: &[crate::manifest::ResolvedDefinition],
) -> Result<()> {
    let package_leaf = manifest.name.rsplit('/').next().unwrap_or(target);
    let artifact_stem = sanitize_artifact_stem(package_leaf);
    let (namespace, manifest_class) = split_class(&manifest.class)?;
    let class_name = format!("{}{}", manifest.class_prefix, manifest_class);

    generate_all(&GenerateArtifacts {
        generated_dir: &root.join(crate::config::GENERATED_DIR),
        artifact_stem: &artifact_stem,
        namespace: &namespace,
        class_name: &class_name,
        library_key,
        symbol_prefix: symbol_prefix_for_library(manifest, library_key),
        alias_class,
        function_prefix: function_prefix.unwrap_or(""),
        native_library_name: &manifest.name,
        native_library_version: &manifest.version,
        description: &manifest.description,
        headers,
        extra_include_dirs,
        dependency_functions,
        exported_symbols,
        definitions,
    })
}

/// Everything `generate_all` needs to emit a package's generated artifacts.
struct GenerateArtifacts<'a> {
    /// `<package>/src/generated` directory the artifacts are written into.
    generated_dir: &'a Path,
    /// File stem for the per-library `*.ffi.php` artifact.
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
    description: &'a str,
    headers: &'a [PathBuf],
    /// Extra `-I` parse dirs from the library's `pkg-config --cflags` (libdir
    /// configs like GLib's `glibconfig.h`/`pango-features.h`).
    extra_include_dirs: &'a [PathBuf],
    /// `C function name -> dependency entity FQCN` for resolving cross-package
    /// calls inside function-like macros (empty for the local `pnlx gen` path).
    dependency_functions: &'a std::collections::BTreeMap<String, String>,
    /// Symbols the resolved native library exports; when present, cdef function
    /// declarations are limited to these. `None` disables the filter.
    exported_symbols: Option<&'a std::collections::BTreeSet<String>>,
    /// `require_definitions` resolved at install time, passed to libclang as `-D`s
    /// and emitted as generated constants.
    definitions: &'a [crate::manifest::ResolvedDefinition],
}

fn generate_all(args: &GenerateArtifacts<'_>) -> Result<()> {
    let generated_dir = args.generated_dir;
    let class_name = args.class_name;
    let out = generated_dir.join(format!(
        "{}{}",
        args.artifact_stem,
        crate::config::FFI_FILE_SUFFIX
    ));
    let (cdef, constants, macro_functions, symbols, symbol_aliases, unsupported_functions, enums) =
        if args.headers.is_empty() {
            (
                read_existing_ffi_cdef(&out)?,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        } else {
            let artifacts = cdef_from_header(
                &read_headers(args.headers)?,
                &HeaderAdapterOptions {
                    symbol_prefix: args
                        .symbol_prefix
                        .clone()
                        .unwrap_or_else(|| symbol_prefix_from_library_key(args.library_key)),
                    entity_fqcn: format!("\\{}\\{}", args.namespace, args.class_name),
                    dependency_functions: args.dependency_functions.clone(),
                    exported_symbols: args.exported_symbols.cloned(),
                    package_header_paths: args.headers.to_vec(),
                    extra_include_dirs: args.extra_include_dirs.to_vec(),
                    definitions: args.definitions.to_vec(),
                },
            )?;
            (
                artifacts.cdef,
                artifacts.constants,
                artifacts.macro_functions,
                artifacts.symbols,
                artifacts.symbol_aliases,
                artifacts.unsupported_functions,
                artifacts.enums,
            )
        };
    // The generated PHP enums, by C name, so signature parsing can tag enum-typed
    // parameters/returns (the wrapper exposes the enum; the dispatched value is int).
    let enum_names: std::collections::BTreeSet<String> =
        enums.iter().map(|def| def.name.clone()).collect();
    // Unsupported (`static inline`) functions become throwing stub methods; they are
    // parsed alongside the cdef for faithful types but never put into the FFI cdef.
    let mut signatures = crate::generate::parse_signatures_with_unsupported(
        &cdef,
        &unsupported_functions,
        &enum_names,
    );
    crate::generate::apply_symbol_aliases(&mut signatures, &symbol_aliases);
    // Field accessors for the struct wrappers come from the cdef's own struct bodies.
    let struct_fields = crate::generate::parse_struct_fields(&cdef);
    generate_ffi_php_from_cdef(&cdef, &out)?;
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
        description: args.description,
        symbols: &symbols,
        enums: &enums,
        struct_fields: &struct_fields,
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
    generate_enums_php(&generated_dir.join("enums"), &template_options)?;
    generate_symbols_php(&generated_dir.join("symbol"), &template_options)?;
    // The method group lives in a `<Class>LibraryComponent` trait, generated in
    // four variants on two axes and selected at runtime by `index.php`:
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
            generate_component_php(
                &dir.join(format!("{class_name}LibraryComponent.php")),
                &template_options,
                allow_cdata,
                scalars_in_return,
            )?;
        }
    }
    // The entity itself is variant-independent (it only `use`s the component trait
    // and carries the metadata attribute), so it is emitted once; `index.php`
    // requires the chosen component variant before it.
    generate_entity_php(
        &generated_dir.join(format!("{class_name}.php")),
        &template_options,
        false,
        false,
    )?;
    generate_const_php(generated_dir, &template_options, &constants)?;
    generate_macro_functions_php(
        &generated_dir.join("macro.functions.php"),
        &template_options,
        &macro_functions,
    )?;
    generate_index_php(&generated_dir.join("index.php"), &template_options)?;
    generate_functions_php(&generated_dir.join("functions.php"), &template_options)?;
    generate_aliases_php(
        &generated_dir.join(crate::config::ALIASES_FILE),
        &signatures,
    )?;
    Ok(())
}

pub(crate) fn read_existing_ffi_cdef(out: &Path) -> Result<String> {
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
    let manifest_path = root.join(crate::config::PNLX_MANIFEST_FILE);
    write_json_if_missing(&manifest_path, &PnlxManifest::default())?;
    crate::ui::success(&format!("initialized {}", manifest_path.display()));
    Ok(())
}

fn publish_pnlx(root: &Path) -> Result<()> {
    let manifest_path = root.join(crate::config::PNLX_MANIFEST_FILE);
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

#[cfg(test)]
mod libclang_gate_tests {
    use super::*;

    #[test]
    fn requires_libclang_when_a_requirement_derives_a_symbol_prefix() {
        // The default manifest's `native` requirement has no explicit prefix, so the
        // generator derives one from the key — meaning libclang is needed to parse it.
        let manifest = PnlxManifest::default();
        assert!(manifest_requires_libclang(&manifest));
        assert!(library_requires_libclang(&manifest, "native"));
    }

    #[test]
    fn skips_libclang_for_an_empty_prefix_verbatim_header() {
        // An explicit empty prefix is the curated verbatim-header path (e.g. libc):
        // the cdef is emitted without libclang, so the preflight must not fire.
        let mut manifest = PnlxManifest::default();
        manifest.requires.get_mut("native").unwrap().symbol_prefix = Some(String::new());
        assert!(!library_requires_libclang(&manifest, "native"));
        assert!(!manifest_requires_libclang(&manifest));
    }
}
