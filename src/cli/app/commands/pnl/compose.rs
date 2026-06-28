//! `pnl compose <members...> --as <Class>`: generate a named composite class that
//! mixes in several installed packages' `<Class>LibraryComponent` traits, so all
//! their functions are exposed through one shared FFI scope (the same effect as
//! `Pnlx\Runtime::compose([...])`, but as a real named class for editor/static
//! analysis). The composite is recorded in `pnl.json` and wired into the autoload.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::package::entity_class_fqn;
use crate::model::manifest::{Composite, PnlManifest, PnlxManifest};
use crate::util::io::{read_json, write_json};

/// One resolved member: its entity FQN, Component trait FQN, the method names it
/// exposes, and its generated `functions.php` (present only when the member was
/// generated with `use_functions`).
struct Member {
    entity_fqn: String,
    trait_fqn: String,
    methods: Vec<String>,
    functions_file: std::path::PathBuf,
}

pub(super) fn compose(
    root: &Path,
    members: &[String],
    as_class: &str,
    prefix: Option<&str>,
) -> Result<()> {
    crate::app::ui::heading("pnl", "compose");

    if members.len() < 2 {
        bail!("compose needs at least two member packages");
    }

    let manifest_path = root.join(crate::model::config::PNL_MANIFEST_FILE);
    let mut manifest: PnlManifest = read_json(&manifest_path)?;
    let workspace = root.join(&manifest.output_dir);
    let packages_root = workspace.join("packages");

    // Resolve each member to its Component trait and method names, de-duplicating
    // by package (PHP rejects the same trait twice; members must be unique).
    let mut canonical_members = Vec::new();
    let mut resolved = Vec::new();
    let mut seen = BTreeSet::new();
    for member in members {
        let (installed, dir) =
            find_installed_package(&packages_root, member)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "package {member} is not installed; run `pnl install {member}` first"
                )
            })?;
        if !seen.insert(installed.name.clone()) {
            continue;
        }
        let entity = entity_class_fqn(&installed).ok_or_else(|| {
            anyhow::anyhow!("package {member} has no namespaced class to compose")
        })?;
        let leaf = entity.rsplit('\\').next().unwrap_or(&entity);
        let generated = dir.join(crate::model::config::GENERATED_DIR);
        resolved.push(Member {
            entity_fqn: entity.clone(),
            trait_fqn: format!("{entity}LibraryComponent"),
            methods: extract_component_methods(
                &generated.join(format!("{leaf}LibraryComponent.php")),
            )?,
            functions_file: generated.join("functions.php"),
        });
        canonical_members.push(installed.name);
    }
    if canonical_members.len() < 2 {
        bail!("compose needs at least two distinct member packages");
    }

    let (uses, resolved_collisions) = build_uses_block(&resolved, prefix)?;
    if !resolved_collisions.is_empty() {
        // The PHP method names are de-conflicted, but co-loaded libraries that
        // export the SAME C symbol resolve to whichever loads first — so a prefixed
        // duplicate dispatches to the first member's implementation, not its own.
        crate::app::ui::warn(&format!(
            "aliased duplicate function(s) ({}) share a C symbol across members; the prefixed copy dispatches to the first member's library",
            resolved_collisions.join(", ")
        ));
    }

    let (namespace, class) = split_class(as_class)?;
    let file = workspace.join("composites").join(format!("{class}.php"));
    fs::create_dir_all(file.parent().expect("composite file has a parent"))
        .with_context(|| format!("failed to create {}", file.display()))?;
    fs::write(&file, render_composite(&namespace, &class, &uses))
        .with_context(|| format!("failed to write {}", file.display()))?;
    crate::app::ui::created("composed", &file);

    // With use_functions, also emit composite global functions under
    // \Pnlx\Func\<Composite>\* that delegate to the composite class (so they share
    // its scope and round-trip out-params), retargeted from each member's own
    // functions.php (which already carries the real, typed signatures).
    if manifest.features.global_functions
        && let Some(functions) = render_composite_functions(&resolved, &namespace, &class, prefix)
    {
        let functions_file = workspace
            .join("composites")
            .join(format!("{class}Functions.php"));
        fs::write(&functions_file, functions)
            .with_context(|| format!("failed to write {}", functions_file.display()))?;
        crate::app::ui::created("composed functions", &functions_file);
    }

    manifest.composites.insert(
        normalize_class(as_class),
        Composite {
            members: canonical_members,
            prefix: prefix.map(str::to_owned),
        },
    );
    write_json(&manifest_path, &manifest)?;

    // Regenerate the autoload so it requires the new composite (after its members).
    super::package::write_pnlx_autoload(root)?;

    crate::app::ui::success(&format!(
        "composed {} from {}",
        as_class,
        members.join(", ")
    ));
    Ok(())
}

/// Build the composite's trait `use` block. With no method-name collisions it is
/// one plain `use` per trait. When members share method names, a single combined
/// `use … { … }` block resolves them: the first member keeps the bare name
/// (`insteadof`), and each later member's clashing method is aliased to
/// `<prefix><method>` — so `--prefix` is required to compose colliding members.
fn build_uses_block(members: &[Member], prefix: Option<&str>) -> Result<(String, Vec<String>)> {
    // method name -> member indices (in order) that expose it.
    let mut by_method: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, member) in members.iter().enumerate() {
        for method in &member.methods {
            let owners = by_method.entry(method.as_str()).or_default();
            if !owners.contains(&index) {
                owners.push(index);
            }
        }
    }
    let collisions: Vec<(&&str, &Vec<usize>)> = by_method
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .collect();

    if collisions.is_empty() {
        let block = members
            .iter()
            .map(|member| format!("    use \\{};", member.trait_fqn))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok((block, Vec::new()));
    }

    let Some(prefix) = prefix else {
        let mut names: Vec<&str> = collisions.iter().map(|(method, _)| **method).collect();
        names.truncate(8);
        bail!(
            "members expose colliding function names ({}{}); pass --prefix to alias the duplicates",
            names.join(", "),
            if collisions.len() > 8 { ", …" } else { "" }
        );
    };

    let trait_list = members
        .iter()
        .map(|member| format!("\\{}", member.trait_fqn))
        .collect::<Vec<_>>()
        .join(", ");

    let mut rules = Vec::new();
    let mut used_aliases = BTreeSet::new();
    for (method, owners) in &collisions {
        let first = &members[owners[0]].trait_fqn;
        let losers = owners[1..]
            .iter()
            .map(|&index| format!("\\{}", members[index].trait_fqn))
            .collect::<Vec<_>>()
            .join(", ");
        rules.push(format!("        \\{first}::{method} insteadof {losers};"));
        for &index in &owners[1..] {
            let mut alias = format!("{prefix}{method}");
            let mut suffix = 2;
            while !used_aliases.insert(alias.clone()) {
                alias = format!("{prefix}{method}{suffix}");
                suffix += 1;
            }
            rules.push(format!(
                "        \\{}::{method} as {alias};",
                members[index].trait_fqn
            ));
        }
    }

    let resolved = collisions
        .iter()
        .map(|(method, _)| (**method).to_owned())
        .collect();
    Ok((
        format!("    use {trait_list} {{\n{}\n    }}", rules.join("\n")),
        resolved,
    ))
}

/// Render the composite class file from a pre-built trait `use` block.
fn render_composite(namespace: &str, class: &str, uses: &str) -> String {
    format!(
        "<?php\n\
\n\
declare(strict_types=1);\n\
\n\
/*\n\
 * !!! DO NOT EDIT THIS FILE !!!\n\
 *\n\
 * Generated by `pnl compose`. It mixes in each member package's\n\
 * `<Class>LibraryComponent` trait so all their functions are exposed through one\n\
 * shared FFI scope (assembled lazily by {{@see \\Pnlx\\Extension\\AbstractExtension}}).\n\
 * Re-run `pnl compose` to regenerate; do not edit by hand.\n\
 */\n\
\n\
namespace {namespace};\n\
\n\
#[\\Pnlx\\Attribute\\AutoGeneratedByPnlx]\n\
class {class} extends \\Pnlx\\Extension\\AbstractExtension\n\
{{\n\
{uses}\n\
}}\n"
    )
}

/// The public static method names a generated Component trait declares, scanned
/// straight from the PHP source (snake- and camel-case variants both count).
fn extract_component_methods(file: &Path) -> Result<Vec<String>> {
    let content =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    const MARKER: &str = "public static function ";
    let mut methods = Vec::new();
    let mut rest = content.as_str();
    while let Some(at) = rest.find(MARKER) {
        rest = &rest[at + MARKER.len()..];
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            methods.push(name);
        }
    }
    Ok(methods)
}

/// Build the composite's `\Pnlx\Func\<Composite>\*` global functions by retargeting
/// each member's generated `functions.php`: keep the real (typed, by-ref) signatures
/// but point the delegation at the composite class and move them under the composite
/// function namespace. Colliding names are renamed with `prefix` (matching the
/// composite class's aliased methods). Returns None when no member exposes functions.
fn render_composite_functions(
    members: &[Member],
    composite_namespace: &str,
    composite_class: &str,
    prefix: Option<&str>,
) -> Option<String> {
    let composite_fqn = format!("{composite_namespace}\\{composite_class}");
    let mut blocks = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut found_any = false;

    for member in members {
        let Ok(content) = fs::read_to_string(&member.functions_file) else {
            continue; // member generated without use_functions
        };
        found_any = true;
        let member_leaf = member
            .entity_fqn
            .rsplit('\\')
            .next()
            .unwrap_or(&member.entity_fqn);

        for chunk in content.split("if (!function_exists(").skip(1) {
            let mut block = format!("if (!function_exists({chunk}");

            // The function name sits in the guard: '…\Func\<member_leaf>\<NAME>'.
            let guard_marker = format!("Func\\\\{member_leaf}\\\\");
            let Some(start) = block.find(&guard_marker).map(|i| i + guard_marker.len()) else {
                continue;
            };
            let Some(len) = block[start..].find('\'') else {
                continue;
            };
            let name = block[start..start + len].to_owned();
            let final_name = if seen.contains(&name) {
                match prefix {
                    Some(prefix) => format!("{prefix}{name}"),
                    None => name.clone(), // unreachable: build_uses_block already errored
                }
            } else {
                name.clone()
            };
            seen.insert(final_name.clone());

            // Delegate to the composite class, under the composite function namespace.
            block = block.replace(
                &format!("\\{}::", member.entity_fqn),
                &format!("\\{composite_fqn}::"),
            );
            block = block.replace(
                &format!("Func\\\\{member_leaf}\\\\"),
                &format!("Func\\\\{composite_class}\\\\"),
            );
            if final_name != name {
                block = block.replace(
                    &format!("function {name}("),
                    &format!("function {final_name}("),
                );
                block = block.replace(&format!("::{name}("), &format!("::{final_name}("));
                block = block.replace(&format!("\\\\{name}'"), &format!("\\\\{final_name}'"));
            }

            blocks.push(block.trim_end().to_owned());
        }
    }

    if !found_any {
        return None;
    }

    Some(format!(
        "<?php\n\
\n\
declare(strict_types=1);\n\
\n\
/*\n\
 * !!! DO NOT EDIT THIS FILE !!!\n\
 *\n\
 * Generated by `pnl compose`. Global helpers for the composed class, under the\n\
 * \\Pnlx\\Func\\{composite_class} namespace, delegating to {composite_fqn} (so they\n\
 * share its FFI scope). Re-run `pnl compose` to regenerate.\n\
 */\n\
\n\
namespace Pnlx\\Func\\{composite_class};\n\
\n\
{}\n",
        blocks.join("\n\n")
    ))
}

/// Split a class FQN into (namespace, class name); the composite must be namespaced.
fn split_class(class: &str) -> Result<(String, String)> {
    let normalized = normalize_class(class);
    let (namespace, name) = normalized.rsplit_once('\\').with_context(|| {
        format!("--as {class} must be a namespaced class, e.g. Pnlx\\Sdlx\\Sdlx")
    })?;
    if namespace.is_empty() || name.is_empty() {
        bail!("--as {class} must be a namespaced class, e.g. Pnlx\\Sdlx\\Sdlx");
    }
    Ok((namespace.to_owned(), name.to_owned()))
}

/// Normalize a class FQN: collapse escaped `\\` and drop any leading separator.
fn normalize_class(class: &str) -> String {
    class
        .replace("\\\\", "\\")
        .trim_start_matches('\\')
        .to_owned()
}

/// Find an installed package (manifest + its version directory) by `vendor/package`
/// name or bare leaf, walking the workspace `packages/` tree.
fn find_installed_package(
    packages_root: &Path,
    member: &str,
) -> Result<Option<(PnlxManifest, PathBuf)>> {
    let mut found = None;
    walk_installed(packages_root, member, &mut found)?;
    Ok(found)
}

fn walk_installed(
    dir: &Path,
    member: &str,
    found: &mut Option<(PnlxManifest, PathBuf)>,
) -> Result<()> {
    if found.is_some() || !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let path = entry
            .with_context(|| format!("failed to read {}", dir.display()))?
            .path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(crate::model::config::PNLX_MANIFEST_FILE);
        if manifest_path.is_file() {
            let manifest = read_json::<PnlxManifest>(&manifest_path)?;
            let leaf = manifest.name.rsplit('/').next().unwrap_or(&manifest.name);
            if manifest.name == member || leaf == member {
                *found = Some((manifest, path));
                return Ok(());
            }
        }
        walk_installed(&path, member, found)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(trait_fqn: &str, methods: &[&str]) -> Member {
        Member {
            entity_fqn: trait_fqn.trim_end_matches("LibraryComponent").to_owned(),
            trait_fqn: trait_fqn.to_owned(),
            methods: methods.iter().map(|m| (*m).to_owned()).collect(),
            functions_file: std::path::PathBuf::new(),
        }
    }

    #[test]
    fn uses_block_is_plain_when_no_methods_collide() {
        let (block, resolved) = build_uses_block(
            &[
                member("Pnlx\\Libsdl\\LibsdlLibraryComponent", &["SDL_Init"]),
                member("Pnlx\\Sdlimage\\SdlimageLibraryComponent", &["IMG_Load"]),
            ],
            None,
        )
        .unwrap();

        assert!(resolved.is_empty());
        assert_eq!(
            block,
            "    use \\Pnlx\\Libsdl\\LibsdlLibraryComponent;\n    use \\Pnlx\\Sdlimage\\SdlimageLibraryComponent;"
        );
    }

    #[test]
    fn colliding_members_require_a_prefix() {
        let members = [
            member("A\\AComponent", &["init"]),
            member("B\\BComponent", &["init"]),
        ];
        let error = build_uses_block(&members, None).unwrap_err().to_string();
        assert!(error.contains("colliding function names"), "{error}");
        assert!(error.contains("init"), "{error}");
    }

    #[test]
    fn prefix_resolves_collisions_with_insteadof_and_as() {
        let (block, resolved) = build_uses_block(
            &[
                member("A\\AComponent", &["init", "a_only"]),
                member("B\\BComponent", &["init"]),
            ],
            Some("b_"),
        )
        .unwrap();

        assert_eq!(resolved, vec!["init".to_owned()]);
        assert!(
            block.contains("use \\A\\AComponent, \\B\\BComponent {"),
            "{block}"
        );
        assert!(
            block.contains("\\A\\AComponent::init insteadof \\B\\BComponent;"),
            "{block}"
        );
        assert!(
            block.contains("\\B\\BComponent::init as b_init;"),
            "{block}"
        );
    }

    #[test]
    fn renders_a_composite_class_from_a_uses_block() {
        let php = render_composite(
            "Pnlx\\Sdlx",
            "Sdlx",
            "    use \\Pnlx\\Libsdl\\LibsdlLibraryComponent;",
        );

        assert!(php.contains("namespace Pnlx\\Sdlx;"), "{php}");
        assert!(
            php.contains("class Sdlx extends \\Pnlx\\Extension\\AbstractExtension"),
            "{php}"
        );
        assert!(
            php.contains("    use \\Pnlx\\Libsdl\\LibsdlLibraryComponent;"),
            "{php}"
        );
        assert!(
            php.contains("#[\\Pnlx\\Attribute\\AutoGeneratedByPnlx]"),
            "{php}"
        );
    }

    #[test]
    fn split_class_requires_a_namespace() {
        assert_eq!(
            split_class("Pnlx\\Sdlx\\Sdlx").unwrap(),
            ("Pnlx\\Sdlx".to_owned(), "Sdlx".to_owned())
        );
        assert_eq!(
            split_class("\\Pnlx\\\\Sdlx\\\\Sdlx").unwrap(),
            ("Pnlx\\Sdlx".to_owned(), "Sdlx".to_owned())
        );
        assert!(split_class("Sdlx").is_err());
    }
}
