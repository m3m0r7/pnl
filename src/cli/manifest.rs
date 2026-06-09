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
    "@pnlx".to_owned()
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
    pub extensions: BTreeMap<String, ExtensionRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlFeatures {
    pub use_functions: bool,
}

/// The default package repository. It is not written into `pnl.json`; pnl keeps
/// it internally as the lowest-priority fallback so bare names like
/// `pnl install libusb` resolve out of the box.
pub const DEFAULT_PACKAGES_REPOSITORY: &str =
    "https://github.com/m3m0r7/pnl-packages/tree/main/packages";

impl Default for PnlManifest {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            repositories: Vec::new(),
            load_paths: Vec::new(),
            output_dir: default_output_dir(),
            features: PnlFeatures::default(),
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
            headers: BTreeMap::new(),
            bridges: BTreeMap::new(),
            requires: BTreeMap::new(),
        }
    }
}
