use std::process::{Command, Stdio};

use chrono::{SecondsFormat, Utc};

use crate::model::manifest::{Platform, PlatformRequirement};

#[derive(Debug, Clone)]
pub struct GeneratedMetadata {
    pub generated_at: String,
    pub host: String,
    pub os: String,
    pub php_version: String,
}

impl GeneratedMetadata {
    pub fn current() -> Self {
        Self {
            generated_at: now(),
            host: generated_host(),
            os: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            php_version: php_platform().0,
        }
    }
}

pub fn current_platform_requirement() -> PlatformRequirement {
    let platform = current_platform();
    PlatformRequirement {
        os: platform.os,
        arch: platform.arch,
        libc: platform.libc,
    }
}

pub fn current_platform() -> Platform {
    let (php_version, php_sapi, php_zts) = php_platform();
    Platform {
        os: match std::env::consts::OS {
            "macos" => "darwin".to_owned(),
            "windows" => "windows".to_owned(),
            other => other.to_owned(),
        },
        arch: match std::env::consts::ARCH {
            "aarch64" => "aarch64".to_owned(),
            "x86_64" => "x86_64".to_owned(),
            other => other.to_owned(),
        },
        libc: current_libc(),
        php_version,
        php_sapi,
        php_zts,
    }
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Whether `current` is covered by a package's declared `platforms`. A declared
/// entry matches when its `os` and `arch` equal the current ones and its `libc`
/// is either unspecified (any C library) or equal to the current `libc`. An
/// empty `platforms` list imposes no restriction.
pub fn platform_supported(
    platforms: &[PlatformRequirement],
    current: &PlatformRequirement,
) -> bool {
    platforms.is_empty()
        || platforms.iter().any(|p| {
            p.os == current.os
                && p.arch == current.arch
                && match (&p.libc, &current.libc) {
                    (Some(declared), Some(actual)) => declared == actual,
                    (Some(_), None) => false,
                    (None, _) => true,
                }
        })
}

/// Human-readable platform, e.g. `linux/aarch64 (musl)`.
pub fn describe_platform(p: &PlatformRequirement) -> String {
    match &p.libc {
        Some(libc) => format!("{}/{} ({libc})", p.os, p.arch),
        None => format!("{}/{}", p.os, p.arch),
    }
}

fn current_libc() -> Option<String> {
    if std::env::consts::OS != "linux" {
        return None;
    }
    if cfg!(target_env = "musl") {
        Some("musl".to_owned())
    } else {
        Some("glibc".to_owned())
    }
}

fn php_platform() -> (String, String, bool) {
    let output = Command::new("php")
        .arg("-r")
        .arg("echo PHP_VERSION, \"\\n\", PHP_SAPI, \"\\n\", PHP_ZTS ? \"1\" : \"0\";")
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else {
        return ("0.0.0".to_owned(), "unknown".to_owned(), false);
    };
    if !output.status.success() {
        return ("0.0.0".to_owned(), "unknown".to_owned(), false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let version = lines.next().unwrap_or("0.0.0").to_owned();
    let sapi = lines.next().unwrap_or("unknown").to_owned();
    let zts = lines.next().unwrap_or("0") == "1";
    (version, sapi, zts)
}

fn generated_host() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let host = gethostname::gethostname()
                .to_string_lossy()
                .trim()
                .to_owned();
            (!host.is_empty()).then_some(host)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(os: &str, arch: &str, libc: Option<&str>) -> PlatformRequirement {
        PlatformRequirement {
            os: os.to_owned(),
            arch: arch.to_owned(),
            libc: libc.map(str::to_owned),
        }
    }

    #[test]
    fn libc_distinguishes_musl_from_glibc() {
        let glibc_only = [req("linux", "aarch64", Some("glibc"))];
        assert!(platform_supported(
            &glibc_only,
            &req("linux", "aarch64", Some("glibc"))
        ));
        // Alpine/musl is not covered by a glibc-only package.
        assert!(!platform_supported(
            &glibc_only,
            &req("linux", "aarch64", Some("musl"))
        ));
    }

    #[test]
    fn unspecified_libc_matches_any_and_arch_os_must_match() {
        let any_linux = [req("linux", "aarch64", None)];
        assert!(platform_supported(
            &any_linux,
            &req("linux", "aarch64", Some("musl"))
        ));
        assert!(!platform_supported(
            &any_linux,
            &req("linux", "x86_64", Some("musl"))
        ));
        assert!(!platform_supported(
            &any_linux,
            &req("darwin", "aarch64", None)
        ));
        // No declared platforms => unrestricted.
        assert!(platform_supported(
            &[],
            &req("linux", "x86_64", Some("musl"))
        ));
    }
}
