use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::SCHEMA_VERSION;
use crate::platform::{current_platform, current_platform_requirement};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Repository {
    #[serde(rename = "type")]
    pub kind: RepositoryType,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Higher values are consulted first when resolving a bare package name.
    /// Defaults to 0; the built-in default repository sits at 0 as a fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryType {
    Git,
    File,
    Https,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionRequirement {
    pub version: String,
    pub required: bool,
}

fn default_output_dir() -> String {
    crate::config::DEFAULT_OUTPUT_DIR.to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlManifest {
    pub schema_version: String,
    pub repositories: Vec<Repository>,
    pub load_paths: Vec<String>,
    /// Directory (relative to the project root) for generated workspace files
    /// — the lock, pathmap, installed packages, and autoload. Defaults to `@pnlx`.
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default)]
    pub features: PnlFeatures,
    /// Per-project overrides for built-in endpoints; omitted when no override
    /// is set.
    #[serde(default, skip_serializing_if = "WorkspaceConfig::is_empty")]
    pub config: WorkspaceConfig,
    pub extensions: BTreeMap<String, ExtensionRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlFeatures {
    pub use_functions: bool,
    /// Expose raw `\FFI\CData` alongside the generated wrapper in entity
    /// signatures (loads the `cdata/<Class>.php` variant). Defaults to false.
    #[serde(default)]
    pub allow_cdata: bool,
    /// Accept a raw PHP scalar (not only a generated wrapper) as a generated
    /// method argument. When false, passing a raw scalar throws at runtime.
    /// Defaults to false.
    #[serde(default)]
    pub use_php_scalars_in_params: bool,
    /// Return PHP native `int`/`float`/`string` for scalars that fit losslessly
    /// (the `scalar/<Class>.php` entity variant) instead of the generated value
    /// wrappers (64-bit unsigned still wrapped). Defaults to false.
    #[serde(default)]
    pub use_php_scalars_in_return: bool,
}

/// Per-project overrides for the built-in service endpoints (see `config.toml`).
/// Absent fields fall back to the values baked into the binary.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceConfig {
    /// Override the repository pnl releases come from (the startup update check
    /// and `pnl self-upgrade`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_repository: Option<String>,
    /// Override the default package registry used as the lowest-priority
    /// fallback when resolving a bare name like `pnl install <name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packages_repository: Option<String>,
}

impl WorkspaceConfig {
    /// Whether no override is set, so `config` can be omitted from `pnl.json`.
    pub fn is_empty(&self) -> bool {
        self.self_repository.is_none() && self.packages_repository.is_none()
    }

    /// The package registry: the workspace override or the built-in default.
    pub fn packages_repository(&self) -> String {
        self.packages_repository
            .clone()
            .unwrap_or_else(|| crate::config::default_packages_repository().to_owned())
    }
}

impl Default for PnlManifest {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            repositories: Vec::new(),
            load_paths: Vec::new(),
            output_dir: default_output_dir(),
            features: PnlFeatures::default(),
            config: WorkspaceConfig::default(),
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub libc: Option<String>,
    pub php_version: String,
    pub php_sapi: String,
    pub php_zts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    #[serde(rename = "type")]
    pub kind: RepositoryType,
    pub url: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dist {
    pub url: String,
    pub sha256: String,
}

/// A repository index (`repository-index.json`) — the catalogue a repository
/// publishes so `pnl find` can list its packages without cloning. Matches the
/// `repository-index` schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryIndex {
    pub schema_version: String,
    pub packages: BTreeMap<String, IndexPackage>,
}

impl RepositoryIndex {
    pub fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            packages: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexPackage {
    /// Alias: when set, this package name redirects to the named package in the
    /// same index (e.g. `sdl` → `ref: "libsdl"`), so `pnl install sdl` resolves
    /// and installs the referenced package.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub versions: BTreeMap<String, IndexPackageVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexPackageVersion {
    /// Path to the package's pnlx.json within the repository (e.g.
    /// `packages/<name>/pnlx.json`).
    pub manifest: String,
    pub dist: Dist,
    pub source: Source,
}

/// A native library as recorded in the lockfile: identity, version, and content
/// hash only. Install-time file paths live in the pathmap, not the lock, so the
/// lock stays portable and path-independent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedNativeLibrary {
    pub name: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedExtension {
    pub version: String,
    pub constraint: String,
    pub source: Source,
    pub dist: Dist,
    /// Fully-qualified generated entity class names this extension exposes, so
    /// `pnl_is_installed(<Class>::class)` resolves straight from the lockfile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    pub dependencies: BTreeMap<String, String>,
    pub requires: BTreeMap<String, LockedNativeLibrary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlLock {
    pub schema_version: String,
    pub generated_at: String,
    pub platform: Platform,
    pub extensions: BTreeMap<String, LockedExtension>,
}

impl PnlLock {
    pub fn empty(platform: Platform) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            generated_at: crate::platform::now(),
            platform,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformRequirement {
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libc: Option<String>,
}

/// A candidate library file name. A plain string is an ordinary on-disk library;
/// the object form `{ "name": "libc.dylib", "virtual": true }` marks a library
/// provided by the system (e.g. libc, which on macOS lives only in the dyld
/// shared cache) — it is linked by name and never required to exist as a file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LibraryName {
    Plain(String),
    Tagged {
        name: String,
        #[serde(rename = "virtual", default)]
        is_virtual: bool,
    },
}

impl LibraryName {
    pub fn name(&self) -> &str {
        match self {
            Self::Plain(name) => name,
            Self::Tagged { name, .. } => name,
        }
    }

    pub fn is_virtual(&self) -> bool {
        matches!(
            self,
            Self::Tagged {
                is_virtual: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeRequirement {
    pub library_names: Vec<LibraryName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_prefix: Option<String>,
    /// Remote source (http/https/ftp/git) to fetch the native library from
    /// instead of searching the local library path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_url: Option<String>,
    /// Remote source (http/https/ftp/git) to fetch the C header from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_url: Option<String>,
    /// C header content embedded directly in the manifest, used as-is for
    /// binding generation when no header file is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_inline: Option<String>,
    pub version: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlxHeader {
    pub name: String,
    pub path: String,
    pub sha256: String,
}

/// One platform's native-dependency installation recipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallationEntry {
    /// Shell command lines that install the native libraries/headers.
    pub install: Vec<String>,
    /// Shell command lines that succeed (exit 0) when the dependency is already
    /// present; if they all pass, installation is skipped.
    #[serde(
        rename = "checkIfExists",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub check_if_exists: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlxManifest {
    pub schema_version: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<Author>,
    pub license: String,
    pub entrypoint: String,
    pub class: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub class_prefix: String,
    /// Optional PHP usage snippets shown when the package finishes installing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// SHA-256 of the install-script material (`installation` commands or the
    /// `self_build` script) stamped by `pnlx publish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_script_hash: Option<String>,
    /// Package-relative script to build/install native dependencies. Mutually
    /// exclusive with `installation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_build: Option<String>,
    /// Optional per-OS commands that install the native dependencies, keyed by
    /// the platform OS name (`darwin`, `linux`, `windows`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub installation: BTreeMap<String, InstallationEntry>,
    #[serde(default)]
    pub headers: Vec<PnlxHeader>,
    pub platforms: Vec<PlatformRequirement>,
    pub requires: BTreeMap<String, NativeRequirement>,
    pub dependencies: BTreeMap<String, ExtensionRequirement>,
}

impl Default for PnlxManifest {
    fn default() -> Self {
        let mut requires = BTreeMap::new();
        requires.insert(
            "native".to_owned(),
            NativeRequirement {
                library_names: vec![
                    LibraryName::Plain("libnative.so".to_owned()),
                    LibraryName::Plain("native.dll".to_owned()),
                ],
                header_names: Vec::new(),
                symbol_prefix: None,
                library_url: None,
                header_url: None,
                header_inline: None,
                version: ">=0.0.0".to_owned(),
                required: true,
            },
        );

        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            name: "example/extension".to_owned(),
            version: "0.1.0".to_owned(),
            description: String::new(),
            authors: Vec::new(),
            license: "MIT".to_owned(),
            entrypoint: "src/generated/index.php".to_owned(),
            class: "Example\\Extension\\Extension".to_owned(),
            class_prefix: String::new(),
            examples: Vec::new(),
            install_script_hash: None,
            self_build: None,
            installation: BTreeMap::new(),
            headers: Vec::new(),
            platforms: vec![current_platform_requirement()],
            requires,
            dependencies: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlxLock {
    pub schema_version: String,
    pub generated_at: String,
    pub dependencies: BTreeMap<String, PnlxLockedDependency>,
}

impl Default for PnlxLock {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            generated_at: crate::platform::now(),
            dependencies: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlxLockedDependency {
    pub version: String,
    pub constraint: String,
    pub source: Source,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedNativeLibrary {
    pub resolved_name: String,
    pub path: String,
    pub version: String,
    pub sha256: String,
    /// RFC3339 timestamp of when this native library was first resolved into the
    /// pathmap, preserved across reinstalls. Optional: virtual/system libraries
    /// and entries not stamped by `pnl install` carry no timestamp (the pathmap
    /// schema and the PHP reader both treat it as nullable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedHeader {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedBridge {
    pub source: String,
    pub library: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlxPathmap {
    pub schema_version: String,
    pub generated_at: String,
    pub platform: Platform,
    /// Lockfile path relative to this pathmap's directory (the workspace),
    /// recorded at install so the generated autoload locates the lock without
    /// assuming a fixed `../` layout.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lock: String,
    /// Absolute path of the `pnl.json` this workspace was generated from, recorded
    /// at install/init so tooling can locate the project even when run from an
    /// unrelated directory. (Runtime resolution itself uses the move-safe
    /// `PNLX_PROJECT_MANIFEST` constant in the generated autoload.)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manifest: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, ResolvedHeader>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bridges: BTreeMap<String, ResolvedBridge>,
    pub requires: BTreeMap<String, ResolvedNativeLibrary>,
}

impl PnlxPathmap {
    pub fn empty_current() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            generated_at: crate::platform::now(),
            platform: current_platform(),
            lock: String::new(),
            manifest: String::new(),
            headers: BTreeMap::new(),
            bridges: BTreeMap::new(),
            requires: BTreeMap::new(),
        }
    }
}
