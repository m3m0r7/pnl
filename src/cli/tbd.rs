//! Reads exported symbols from a `.tbd` (text-based dylib stub).
//!
//! On modern macOS the system dylibs (libSystem, and thus libc) have no on-disk
//! `.dylib` file — they live in the dyld shared cache — but the SDK ships a `.tbd`
//! stub that declares their exported symbols. A `.tbd` is a YAML document stream
//! tagged `!tapi-tbd`; the tag is a YAML application tag (not part of the data), so
//! once it is stripped the body parses with an ordinary YAML parser.
//!
//! This lets the export-symbol filter run on macOS too (the runtime still `dlopen`s
//! the dylib by its `install-name`, which dyld resolves from the shared cache).

use std::collections::BTreeSet;

use yaml_rust2::YamlLoader;

/// What a `.tbd` declares: the exported symbol names (Mach-O leading `_` stripped,
/// to match C symbol names in the cdef) and the `install-name` — the path the
/// runtime should `dlopen` (the `.tbd` itself is never loaded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TbdExports {
    pub install_name: Option<String>,
    pub symbols: BTreeSet<String>,
}

/// Parse a `.tbd`'s contents into its exported symbols and install-name, or `None`
/// if it is not parseable / declares no symbols.
pub fn parse_tbd(content: &str) -> Option<TbdExports> {
    let cleaned = strip_tapi_tags(content);
    let documents = YamlLoader::load_from_str(&cleaned).ok()?;
    let mut symbols = BTreeSet::new();
    let mut install_name = None;
    for document in &documents {
        // `.tbd` is a stream: the umbrella document plus one per re-exported
        // sub-library, all carrying symbols. Use the first install-name (the
        // umbrella's) as the runtime load target.
        if install_name.is_none()
            && let Some(name) = document["install-name"].as_str()
        {
            install_name = Some(name.to_owned());
        }
        for section in ["exports", "reexports"] {
            let Some(entries) = document[section].as_vec() else {
                continue;
            };
            for entry in entries {
                for key in ["symbols", "weak-symbols"] {
                    let Some(list) = entry[key].as_vec() else {
                        continue;
                    };
                    for item in list {
                        if let Some(symbol) = item.as_str() {
                            symbols.insert(strip_leading_underscore(symbol));
                        }
                    }
                }
            }
        }
    }
    (!symbols.is_empty()).then_some(TbdExports {
        install_name,
        symbols,
    })
}

/// Replace the `!tapi-tbd*` application tag on each document marker with a bare
/// `---`. A generic YAML parser would otherwise need a constructor for the tag; the
/// document body itself is ordinary YAML.
fn strip_tapi_tags(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("---") && trimmed.contains("!tapi") {
                "---"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip the single Mach-O leading underscore C symbols carry (`_printf` →
/// `printf`), so the names line up with the cdef's C function names.
fn strip_leading_underscore(symbol: &str) -> String {
    symbol.strip_prefix('_').unwrap_or(symbol).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tapi_tbd_v4_symbols_and_install_name() {
        // A minimal two-document stream in the shape Apple's SDK `.tbd` uses. A raw
        // string preserves the YAML indentation (a `\n\` continuation would strip
        // the next line's leading whitespace and break the structure).
        let tbd = r#"--- !tapi-tbd
tbd-version:     4
targets:         [ arm64-macos, x86_64-macos ]
install-name:    '/usr/lib/libSystem.B.dylib'
exports:
  - targets:         [ arm64-macos ]
    symbols:         [ _printf, _malloc, 'R8289209$_close' ]
    weak-symbols:    [ _weakfn ]
--- !tapi-tbd
tbd-version:     4
targets:         [ arm64-macos ]
install-name:    '/usr/lib/system/libsystem_c.dylib'
exports:
  - targets:         [ arm64-macos ]
    symbols:         [ _atexit, _puts ]
"#;
        let parsed = parse_tbd(tbd).expect("tbd parses");
        assert_eq!(
            parsed.install_name.as_deref(),
            Some("/usr/lib/libSystem.B.dylib")
        );
        // Leading Mach-O underscore stripped; weak symbols and later documents
        // included; objc/aliased forms kept verbatim (they just won't match a cdef).
        assert!(parsed.symbols.contains("printf"));
        assert!(parsed.symbols.contains("malloc"));
        assert!(parsed.symbols.contains("weakfn"));
        assert!(parsed.symbols.contains("atexit"));
        assert!(parsed.symbols.contains("puts"));
        assert!(!parsed.symbols.contains("_printf"));
    }

    #[test]
    fn non_tbd_input_yields_none() {
        assert!(parse_tbd("this is not yaml at all: [unterminated").is_none());
        assert!(parse_tbd("--- !tapi-tbd\ntbd-version: 4\n").is_none());
    }
}
