pub mod commands;
pub mod generate;
pub mod git_source;
pub mod header_adapter;
pub mod io;
pub mod manifest;
pub mod platform;
pub mod schema;
pub mod validate;

pub const SCHEMA_VERSION: &str = "2026-07-01";

#[cfg(test)]
mod tests {
    use crate::SCHEMA_VERSION;
    use crate::git_source::GitSource;
    use crate::io::{read_json, write_json_if_missing};
    use crate::manifest::PnlxManifest;
    use crate::validate::{
        validate_package_name, validate_pnlx_manifest_values, validate_rfc3339_datetime,
        validate_schema_version, validate_semver, validate_version_constraint,
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
            GitSource::parse("https://github.com/m3m0r7/pnl-packages/packages/libusb").unwrap();
        assert_eq!(source.url, "https://github.com/m3m0r7/pnl-packages.git");
        assert_eq!(source.package_name(), "m3m0r7/pnl-packages");
        assert_eq!(source.branch, None);
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/libusb")
        );
    }

    #[test]
    fn parses_github_tree_source_with_package_subpath() {
        let source =
            GitSource::parse("https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb")
                .unwrap();
        assert_eq!(source.url, "https://github.com/m3m0r7/pnl-packages.git");
        assert_eq!(source.package_name(), "m3m0r7/pnl-packages");
        assert_eq!(source.branch.as_deref(), Some("main"));
        assert_eq!(
            source.package_path,
            std::path::PathBuf::from("packages/libusb")
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
    fn parses_scp_like_git_source_with_unusual_host() {
        let source = GitSource::parse("git@git@github.com:xxxxx/zzzzz/path/to/").unwrap();
        assert_eq!(source.url, "git@git@github.com:xxxxx/zzzzz.git");
        assert_eq!(source.package_name(), "xxxxx/zzzzz");
        assert_eq!(source.branch, None);
        assert_eq!(source.package_path, std::path::PathBuf::from("path/to"));
    }

    #[test]
    fn parses_scp_like_git_source_root_repository() {
        let source = GitSource::parse("git@github.com:sunng87/handlebars-rust.git").unwrap();
        assert_eq!(source.url, "git@github.com:sunng87/handlebars-rust.git");
        assert_eq!(source.package_name(), "sunng87/handlebars-rust");
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
    fn default_pnlx_manifest_has_schema_version() {
        assert_eq!(PnlxManifest::default().schema_version, SCHEMA_VERSION);
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
        assert!(validate_semver("1.2").is_err());
        assert!(validate_semver("1.2.x").is_err());
        assert!(validate_semver("1.2.3-").is_err());
    }

    #[test]
    fn validates_version_constraints() {
        validate_version_constraint("1.2.3").unwrap();
        validate_version_constraint("=1.2.3").unwrap();
        validate_version_constraint(">=1.2.0 <2.0.0").unwrap();
        validate_version_constraint("^1.2.3").unwrap();
        validate_version_constraint("~1.2.3").unwrap();
        assert!(validate_version_constraint("").is_err());
        assert!(validate_version_constraint(" >=1.2.3").is_err());
        assert!(validate_version_constraint(">=1.2").is_err());
    }

    #[test]
    fn rejects_pnlx_manifest_without_native_requirements() {
        let mut manifest = PnlxManifest::default();
        manifest.requires.clear();

        assert!(validate_pnlx_manifest_values(&manifest).is_err());
    }

    #[test]
    fn init_writes_pnlx_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pnlx.json");
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
