/// The compiled support `cdylib` embedded at build time (see `build.rs`). Empty
/// on a single-pass build; populated by the release two-pass build. `pnl install`
/// writes it into `@pnlx/runtime`, falling back to the cdylib next to the
/// executable when this is empty.
pub const SUPPORT_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/support.lib"));

pub mod about;
pub mod archive;
pub mod cache;
pub mod commands;
pub mod config;
pub mod fetch;
pub mod ffi;
pub mod generate;
pub mod git_source;
pub mod glob;
pub mod header_adapter;
pub mod highlight;
pub mod install_script;
pub mod interaction;
pub mod io;
pub mod manifest;
pub mod pkg_config;
pub mod platform;
pub mod release;
pub mod repository_index;
pub mod schema;
pub mod sdk_assets;
pub mod self_upgrade;
pub mod ui;
pub mod validate;
pub mod version;
pub mod workspace;

pub use crate::config::SCHEMA_VERSION;

#[cfg(test)]
mod tests {
    use crate::SCHEMA_VERSION;
    use crate::git_source::GitSource;
    use crate::io::{read_json, write_json_if_missing};
    use crate::manifest::PnlxManifest;
    use crate::validate::{
        validate_package_name, validate_pnlx_manifest_values, validate_rfc3339_datetime,
        validate_schema_version, validate_semver, validate_sha256, validate_version_constraint,
    };

    #[test]
    fn parses_https_github_source() {
        let source = GitSource::parse("https://github.com/example/native-ext").unwrap();
        assert_eq!(source.url, "https://github.com/example/native-ext.git");
        assert_eq!(source.package_name(), "example/native-ext");
        assert_eq!(source.branch, None);
        assert!(source.package_path.as_os_str().is_empty());
    }

    #[test]
    fn parses_github_source_with_package_subpath() {
        let source =
            GitSource::parse("https://github.com/m3m0r7/pnl-packages/packages/widget").unwrap();
        assert_eq!(source.url, "https://github.com/m3m0r7/pnl-packages.git");
        assert_eq!(source.package_name(), "m3m0r7/pnl-packages");
        assert_eq!(source.branch, None);
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/widget")
        );
    }

    #[test]
    fn parses_github_tree_source_with_package_subpath() {
        let source =
            GitSource::parse("https://github.com/m3m0r7/pnl-packages/tree/main/packages/widget")
                .unwrap();
        assert_eq!(source.url, "https://github.com/m3m0r7/pnl-packages.git");
        assert_eq!(source.package_name(), "m3m0r7/pnl-packages");
        assert_eq!(source.branch.as_deref(), Some("main"));
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/widget")
        );
    }

    #[test]
    fn parses_ssh_github_source() {
        let source = GitSource::parse("git@github.com:example/native-ext.git").unwrap();
        assert_eq!(source.url, "git@github.com:example/native-ext.git");
        assert_eq!(source.package_name(), "example/native-ext");
        assert_eq!(source.branch, None);
        assert!(source.package_path.as_os_str().is_empty());
    }

    #[test]
    fn parses_scp_like_git_source_with_package_subpath() {
        let source = GitSource::parse("git@github.com:xxxxx/zzzzz/path/to").unwrap();
        assert_eq!(source.url, "git@github.com:xxxxx/zzzzz.git");
        assert_eq!(source.package_name(), "xxxxx/zzzzz");
        assert_eq!(source.branch, None);
        assert_eq!(source.package_path, std::path::PathBuf::from("path/to"));
    }

    #[test]
    fn rejects_malformed_scp_like_git_source() {
        // A doubled `git@git@…` user component is not a valid scp-like git URL.
        assert!(GitSource::parse("git@git@github.com:xxxxx/zzzzz/path/to/").is_err());
    }

    #[test]
    fn parses_scp_like_git_source_root_repository() {
        let source = GitSource::parse("git@github.com:m3m0r7/pnl-packages.git").unwrap();
        assert_eq!(source.url, "git@github.com:m3m0r7/pnl-packages.git");
        assert_eq!(source.package_name(), "m3m0r7/pnl-packages");
        assert_eq!(source.branch, None);
        assert!(source.package_path.as_os_str().is_empty());
    }

    #[test]
    fn parses_generic_git_source() {
        let source = GitSource::parse("https://git.example.test/vendor/native-ext.git").unwrap();
        assert_eq!(source.url, "https://git.example.test/vendor/native-ext.git");
        assert_eq!(source.package_name(), "vendor/native-ext");
        assert_eq!(source.branch, None);
    }

    #[test]
    fn parses_github_tree_source_at_repository_root() {
        let source = GitSource::parse("https://github.com/example/native-ext/tree/master").unwrap();
        assert_eq!(source.url, "https://github.com/example/native-ext.git");
        assert_eq!(source.package_name(), "example/native-ext");
        assert_eq!(source.branch.as_deref(), Some("master"));
        assert!(source.package_path.as_os_str().is_empty());
    }

    #[test]
    fn rejects_github_blob_source() {
        assert!(
            GitSource::parse("https://github.com/example/native-ext/blob/main/pnlx.json").is_err()
        );
    }

    #[test]
    fn parses_gitlab_tree_source_with_package_subpath() {
        let source =
            GitSource::parse("https://gitlab.com/group/project/-/tree/main/packages/widget")
                .unwrap();
        assert_eq!(source.url, "https://gitlab.com/group/project.git");
        assert_eq!(source.package_name(), "group/project");
        assert_eq!(source.branch.as_deref(), Some("main"));
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/widget")
        );
    }

    #[test]
    fn parses_gitlab_root_source() {
        let source = GitSource::parse("https://gitlab.com/group/project").unwrap();
        assert_eq!(source.url, "https://gitlab.com/group/project.git");
        assert_eq!(source.branch, None);
        assert!(source.package_path.as_os_str().is_empty());
    }

    #[test]
    fn parses_bitbucket_src_source_with_package_subpath() {
        let source =
            GitSource::parse("https://bitbucket.org/team/repo/src/develop/packages/lib").unwrap();
        assert_eq!(source.url, "https://bitbucket.org/team/repo.git");
        assert_eq!(source.package_name(), "team/repo");
        assert_eq!(source.branch.as_deref(), Some("develop"));
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/lib")
        );
    }

    #[test]
    fn parses_unknown_host_web_url_as_generic() {
        let source = GitSource::parse("https://git.example.test/vendor/native-ext").unwrap();
        assert_eq!(source.url, "https://git.example.test/vendor/native-ext.git");
        assert_eq!(source.package_name(), "vendor/native-ext");
        assert_eq!(source.branch, None);

        let with_subpath =
            GitSource::parse("https://git.example.test/vendor/native-ext/packages/lib").unwrap();
        assert_eq!(
            with_subpath.package_path,
            std::path::PathBuf::from("packages/lib")
        );
    }

    #[test]
    fn default_pnlx_manifest_has_schema_version() {
        assert_eq!(PnlxManifest::default().schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn parses_plain_and_virtual_library_names() {
        use crate::manifest::LibraryName;
        let names: Vec<LibraryName> = serde_json::from_str(
            r#"["widget-1.0.dylib", {"name": "libc.dylib", "virtual": true}, {"name": "x.so"}]"#,
        )
        .unwrap();
        assert_eq!(names[0].name(), "widget-1.0.dylib");
        assert!(!names[0].is_virtual());
        assert_eq!(names[1].name(), "libc.dylib");
        assert!(names[1].is_virtual());
        // The object form without `virtual` defaults to non-virtual.
        assert_eq!(names[2].name(), "x.so");
        assert!(!names[2].is_virtual());
        // A plain string round-trips back to a JSON string, not an object.
        assert_eq!(
            serde_json::to_string(&names[0]).unwrap(),
            "\"widget-1.0.dylib\""
        );
    }

    #[test]
    fn default_pnl_manifest_has_no_repositories() {
        use crate::manifest::PnlManifest;
        // The default repository is kept internally, not written into pnl.json.
        let manifest = PnlManifest::default();
        assert!(manifest.repositories.is_empty());
        assert!(!manifest.features.use_functions);
    }

    #[test]
    fn validates_package_names() {
        validate_package_name("vendor/native-ext").unwrap();
        assert!(validate_package_name("native-ext").is_err());
        assert!(validate_package_name("Vendor/native-ext").is_ok());
    }

    #[test]
    fn validates_semver_values() {
        validate_semver("1.2.3").unwrap();
        validate_semver("1.2.3-alpha.1").unwrap();
        validate_semver("1.2.3+build.1").unwrap();
        // Real C libraries omit the patch (74.2) or use a bare date (20190702).
        validate_semver("1.5").unwrap();
        validate_semver("74.2").unwrap();
        validate_semver("20190702").unwrap();
        assert!(validate_semver("1.2.x").is_err());
        assert!(validate_semver("1.2.3-").is_err());
        assert!(validate_semver("").is_err());
    }

    #[test]
    fn validates_version_constraints() {
        validate_version_constraint("1.2.3").unwrap();
        validate_version_constraint("=1.2.3").unwrap();
        validate_version_constraint(">=1.2.0 & <2.0.0").unwrap();
        validate_version_constraint(">=1.2.0 & <2.0.0 | >=3.0.0").unwrap();
        validate_version_constraint("^1.2.3").unwrap();
        validate_version_constraint("~1.2.3").unwrap();
        assert!(validate_version_constraint("").is_err());
        assert!(validate_version_constraint(">=1.2.0 <2.0.0").is_err());
        assert!(validate_version_constraint(">=1.2").is_err());
    }

    #[test]
    fn validates_example_paths_are_package_relative() {
        use crate::validate::validate_relative_package_path;
        validate_relative_package_path("examples", "EXAMPLES.md").unwrap();
        validate_relative_package_path("examples", "docs/EXAMPLES.md").unwrap();
        assert!(validate_relative_package_path("examples", "/etc/passwd").is_err());
        assert!(validate_relative_package_path("examples", "../outside.md").is_err());
        assert!(validate_relative_package_path("examples", "docs\\EXAMPLES.md").is_err());
    }

    #[test]
    fn validates_install_script_hashes() {
        validate_sha256(&"a".repeat(64)).unwrap();
        assert!(validate_sha256("not-a-sha").is_err());
    }

    #[test]
    fn rejects_pnlx_manifest_without_native_requirements() {
        let mut manifest = PnlxManifest::default();
        manifest.requires.clear();

        assert!(validate_pnlx_manifest_values(&manifest).is_err());
    }

    #[test]
    fn rejects_self_build_together_with_installation() {
        use crate::manifest::InstallationEntry;
        let mut manifest = PnlxManifest {
            self_build: Some("build.sh".to_owned()),
            ..PnlxManifest::default()
        };
        manifest.installation.insert(
            "linux".to_owned(),
            InstallationEntry {
                install: vec!["apt-get install libexample-dev".to_owned()],
                check_if_exists: Vec::new(),
            },
        );

        assert!(validate_pnlx_manifest_values(&manifest).is_err());
    }

    #[test]
    fn init_writes_pnlx_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(crate::config::PNLX_MANIFEST_FILE);
        write_json_if_missing(&path, &PnlxManifest::default()).unwrap();
        let manifest = read_json::<PnlxManifest>(&path).unwrap();
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn validates_schema_version_as_supported_date() {
        validate_schema_version(SCHEMA_VERSION).unwrap();
        assert!(validate_schema_version("2026-99-99").is_err());
        assert!(validate_schema_version("2026-07-02").is_err());
    }

    #[test]
    fn validates_generated_at_as_rfc3339_datetime() {
        validate_rfc3339_datetime("generated_at", "2026-07-01T00:00:00Z").unwrap();
        assert!(validate_rfc3339_datetime("generated_at", "2026-07-01").is_err());
    }
}
