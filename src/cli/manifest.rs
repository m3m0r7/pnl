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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlManifest {
    pub schema_version: String,
    pub repositories: Vec<Repository>,
    pub load_paths: Vec<String>,
    #[serde(default)]
    pub enables: PnlEnables,
    pub extensions: BTreeMap<String, ExtensionRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PnlEnables {
    pub use_functions: bool,
}

impl Default for PnlManifest {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            repositories: Vec::new(),
            load_paths: Vec::new(),
            enables: PnlEnables::default(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedNativeLibrary {
    pub name: String,
    pub version: String,
    pub path: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeRequirement {
    pub library_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_prefix: Option<String>,
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
                library_names: vec!["libnative.so".to_owned(), "native.dll".to_owned()],
                header_names: Vec::new(),
                symbol_prefix: None,
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
