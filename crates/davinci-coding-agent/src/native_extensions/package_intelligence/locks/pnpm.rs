use super::{LockedPackage, LockfileData, LockfileKind};
use std::collections::BTreeMap;

pub fn parse(content: &str) -> Result<LockfileData, String> {
    let mut data = LockfileData {
        kind: LockfileKind::Pnpm,
        lockfile_version: None,
        packages: BTreeMap::new(),
        workspace_packages: BTreeMap::new(),
    };

    let mut current_section = "";
    let mut current_importer = "";
    let mut in_importer_deps = false;
    let mut current_importer_pkg = String::new();
    let mut current_importer_spec = String::new();
    let mut current_importer_ver = String::new();

    let mut current_pkg_name = String::new();
    let mut current_pkg_version = String::new();
    let mut current_pkg_integrity = None;
    let mut current_pkg_deps = BTreeMap::new();
    let mut in_pkg_deps = false;

    let flush_importer_dep =
        |data: &mut LockfileData, importer: &str, pkg: &str, spec: &str, ver: &str| {
            if !importer.is_empty() && !pkg.is_empty() {
                let resolved_ver = ver.to_string();
                if resolved_ver.starts_with("link:") {
                    // Workspace link
                }
                let locked = LockedPackage {
                    name: pkg.to_string(),
                    version: resolved_ver,
                    resolved: Some(spec.to_string()),
                    integrity: None,
                    dependencies: BTreeMap::new(),
                };
                data.workspace_packages
                    .entry(importer.to_string())
                    .or_default()
                    .insert(pkg.to_string(), locked);
            }
        };

    let flush_pkg = |data: &mut LockfileData,
                     name: &str,
                     ver: &str,
                     integrity: Option<String>,
                     deps: BTreeMap<String, String>| {
        if !name.is_empty() && !ver.is_empty() {
            data.packages
                .entry(name.to_string())
                .or_insert_with(|| LockedPackage {
                    name: name.to_string(),
                    version: ver.to_string(),
                    resolved: None,
                    integrity,
                    dependencies: deps,
                });
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("lockfileVersion:") {
            let ver = trimmed
                .strip_prefix("lockfileVersion:")
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"']);
            data.lockfile_version = Some(ver.to_string());
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        if indent == 0 {
            if current_section == "packages" {
                flush_pkg(
                    &mut data,
                    &current_pkg_name,
                    &current_pkg_version,
                    current_pkg_integrity.take(),
                    std::mem::take(&mut current_pkg_deps),
                );
                current_pkg_name.clear();
                current_pkg_version.clear();
            } else if current_section == "importers" {
                flush_importer_dep(
                    &mut data,
                    current_importer,
                    &current_importer_pkg,
                    &current_importer_spec,
                    &current_importer_ver,
                );
                current_importer_pkg.clear();
                current_importer_spec.clear();
                current_importer_ver.clear();
            }

            if trimmed == "importers:" {
                current_section = "importers";
            } else if trimmed == "packages:" {
                current_section = "packages";
            } else {
                current_section = "";
            }
            continue;
        }

        if current_section == "importers" {
            if indent == 2 {
                flush_importer_dep(
                    &mut data,
                    current_importer,
                    &current_importer_pkg,
                    &current_importer_spec,
                    &current_importer_ver,
                );
                current_importer_pkg.clear();
                current_importer_spec.clear();
                current_importer_ver.clear();
                current_importer = trimmed.trim_end_matches(':');
                in_importer_deps = false;
            } else if indent == 4 {
                flush_importer_dep(
                    &mut data,
                    current_importer,
                    &current_importer_pkg,
                    &current_importer_spec,
                    &current_importer_ver,
                );
                current_importer_pkg.clear();
                current_importer_spec.clear();
                current_importer_ver.clear();
                in_importer_deps = matches!(
                    trimmed,
                    "dependencies:" | "devDependencies:" | "optionalDependencies:"
                );
            } else if indent == 6 && in_importer_deps {
                flush_importer_dep(
                    &mut data,
                    current_importer,
                    &current_importer_pkg,
                    &current_importer_spec,
                    &current_importer_ver,
                );
                current_importer_pkg = trimmed
                    .trim_end_matches(':')
                    .trim_matches(['\'', '"'])
                    .to_string();
                current_importer_spec.clear();
                current_importer_ver.clear();
            } else if indent >= 8 && in_importer_deps {
                if let Some(spec) = trimmed.strip_prefix("specifier:") {
                    current_importer_spec = spec.trim().trim_matches(['\'', '"']).to_string();
                } else if let Some(ver) = trimmed.strip_prefix("version:") {
                    current_importer_ver = ver.trim().trim_matches(['\'', '"']).to_string();
                }
            }
        } else if current_section == "packages" {
            if indent == 2 {
                flush_pkg(
                    &mut data,
                    &current_pkg_name,
                    &current_pkg_version,
                    current_pkg_integrity.take(),
                    std::mem::take(&mut current_pkg_deps),
                );
                current_pkg_name.clear();
                current_pkg_version.clear();
                in_pkg_deps = false;

                let raw_pkg = trimmed.trim_end_matches(':').trim_matches(['\'', '"']);
                let clean = raw_pkg.strip_prefix('/').unwrap_or(raw_pkg);
                // Parse package name and version: handle scoped @scope/pkg@1.0.0 vs pkg@1.0.0
                let (name, ver) = if let Some(stripped) = clean.strip_prefix('@') {
                    if let Some((scope_pkg, ver)) = stripped.split_once('@') {
                        (format!("@{}", scope_pkg), ver)
                    } else {
                        (clean.to_string(), "")
                    }
                } else if let Some((p, v)) = clean.split_once('@') {
                    (p.to_string(), v)
                } else {
                    (clean.to_string(), "")
                };
                let clean_ver = ver.split('(').next().unwrap_or(ver).trim();
                current_pkg_name = name;
                current_pkg_version = clean_ver.to_string();
            } else if indent == 4 {
                in_pkg_deps = trimmed == "dependencies:";
                if let Some(res) = trimmed.strip_prefix("resolution:") {
                    if let Some(pos) = res.find("integrity:") {
                        let integ = res[pos..]
                            .strip_prefix("integrity:")
                            .unwrap_or("")
                            .trim()
                            .trim_matches(['}', ' ']);
                        current_pkg_integrity = Some(integ.to_string());
                    }
                }
            } else if indent >= 6 && in_pkg_deps {
                if let Some((dep_name, dep_ver)) = trimmed.split_once(':') {
                    let dname = dep_name.trim().trim_matches(['\'', '"']).to_string();
                    let dver = dep_ver.trim().trim_matches(['\'', '"']).to_string();
                    current_pkg_deps.insert(dname, dver);
                }
            }
        }
    }

    if current_section == "packages" {
        flush_pkg(
            &mut data,
            &current_pkg_name,
            &current_pkg_version,
            current_pkg_integrity.take(),
            std::mem::take(&mut current_pkg_deps),
        );
    } else if current_section == "importers" {
        flush_importer_dep(
            &mut data,
            current_importer,
            &current_importer_pkg,
            &current_importer_spec,
            &current_importer_ver,
        );
    }

    Ok(data)
}
