use super::{LockedPackage, LockfileData, LockfileKind};
use std::collections::BTreeMap;

pub fn parse(content: &str) -> Result<LockfileData, String> {
    let is_berry = content.contains("__metadata:") || content.contains("@npm:");
    if is_berry {
        parse_berry(content)
    } else {
        parse_classic(content)
    }
}

fn parse_classic(content: &str) -> Result<LockfileData, String> {
    let mut data = LockfileData {
        kind: LockfileKind::YarnClassic,
        lockfile_version: Some("1".to_string()),
        packages: BTreeMap::new(),
        workspace_packages: BTreeMap::new(),
    };

    let mut current_names = Vec::new();
    let mut current_version = String::new();
    let mut current_resolved = None;
    let mut current_integrity = None;
    let mut current_deps = BTreeMap::new();
    let mut in_deps = false;

    let flush = |data: &mut LockfileData,
                 names: &mut Vec<String>,
                 ver: &str,
                 res: Option<String>,
                 integ: Option<String>,
                 deps: BTreeMap<String, String>| {
        if !ver.is_empty() {
            for name in names.drain(..) {
                data.packages
                    .entry(name.clone())
                    .or_insert_with(|| LockedPackage {
                        name,
                        version: ver.to_string(),
                        resolved: res.clone(),
                        integrity: integ.clone(),
                        dependencies: deps.clone(),
                    });
            }
        }
        names.clear();
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if indent == 0 && trimmed.ends_with(':') {
            flush(
                &mut data,
                &mut current_names,
                &current_version,
                current_resolved.take(),
                current_integrity.take(),
                std::mem::take(&mut current_deps),
            );
            current_version.clear();
            in_deps = false;

            let header = trimmed.trim_end_matches(':');
            for item in header.split(',') {
                let clean = item.trim().trim_matches(['"', '\'']);
                let name = if let Some(stripped) = clean.strip_prefix('@') {
                    if let Some((scope_pkg, _)) = stripped.split_once('@') {
                        format!("@{}", scope_pkg)
                    } else {
                        clean.to_string()
                    }
                } else if let Some((n, _)) = clean.split_once('@') {
                    n.to_string()
                } else {
                    clean.to_string()
                };
                if !name.is_empty() && !current_names.contains(&name) {
                    current_names.push(name);
                }
            }
        } else if indent >= 2 {
            if trimmed.starts_with("version ") {
                let ver = trimmed
                    .strip_prefix("version ")
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'']);
                current_version = ver.to_string();
            } else if trimmed.starts_with("resolved ") {
                let res = trimmed
                    .strip_prefix("resolved ")
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'']);
                current_resolved = Some(res.to_string());
            } else if trimmed.starts_with("integrity ") {
                let integ = trimmed
                    .strip_prefix("integrity ")
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'']);
                current_integrity = Some(integ.to_string());
            } else if trimmed == "dependencies:" || trimmed == "optionalDependencies:" {
                in_deps = true;
            } else if in_deps && indent >= 4 {
                if let Some((dname, dver)) = trimmed.split_once(' ') {
                    let dn = dname.trim().trim_matches(['"', '\'']);
                    let dv = dver.trim().trim_matches(['"', '\'']);
                    current_deps.insert(dn.to_string(), dv.to_string());
                }
            }
        }
    }

    flush(
        &mut data,
        &mut current_names,
        &current_version,
        current_resolved.take(),
        current_integrity.take(),
        std::mem::take(&mut current_deps),
    );
    Ok(data)
}

fn parse_berry(content: &str) -> Result<LockfileData, String> {
    let mut data = LockfileData {
        kind: LockfileKind::YarnBerry,
        lockfile_version: Some("berry".to_string()),
        packages: BTreeMap::new(),
        workspace_packages: BTreeMap::new(),
    };

    let mut current_name = String::new();
    let mut current_version = String::new();
    let mut current_resolution = None;
    let mut current_checksum = None;
    let mut current_deps = BTreeMap::new();
    let mut in_deps = false;

    let flush = |data: &mut LockfileData,
                 name: &str,
                 ver: &str,
                 res: Option<String>,
                 integ: Option<String>,
                 deps: BTreeMap<String, String>| {
        if !name.is_empty() && !ver.is_empty() {
            data.packages
                .entry(name.to_string())
                .or_insert_with(|| LockedPackage {
                    name: name.to_string(),
                    version: ver.to_string(),
                    resolved: res,
                    integrity: integ,
                    dependencies: deps,
                });
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if indent == 0 && trimmed.ends_with(':') {
            flush(
                &mut data,
                &current_name,
                &current_version,
                current_resolution.take(),
                current_checksum.take(),
                std::mem::take(&mut current_deps),
            );
            current_name.clear();
            current_version.clear();
            in_deps = false;

            let header = trimmed.trim_end_matches(':').trim_matches(['"', '\'']);
            if header.starts_with("__metadata") {
                continue;
            }
            let name = if let Some(stripped) = header.strip_prefix('@') {
                if let Some((scope_pkg, _)) = stripped.split_once('@') {
                    format!("@{}", scope_pkg)
                } else {
                    header.to_string()
                }
            } else if let Some((n, _)) = header.split_once('@') {
                n.to_string()
            } else {
                header.to_string()
            };
            current_name = name;
        } else if indent >= 2 {
            if let Some(ver) = trimmed.strip_prefix("version:") {
                current_version = ver.trim().trim_matches(['"', '\'']).to_string();
            } else if let Some(res) = trimmed.strip_prefix("resolution:") {
                current_resolution = Some(res.trim().trim_matches(['"', '\'']).to_string());
            } else if let Some(cs) = trimmed.strip_prefix("checksum:") {
                current_checksum = Some(cs.trim().trim_matches(['"', '\'']).to_string());
            } else if trimmed == "dependencies:" || trimmed == "peerDependencies:" {
                in_deps = true;
            } else if in_deps && indent >= 4 {
                if let Some((dname, dver)) = trimmed.split_once(':') {
                    let dn = dname.trim().trim_matches(['"', '\'']);
                    let dv = dver.trim().trim_matches(['"', '\'']);
                    current_deps.insert(dn.to_string(), dv.to_string());
                }
            }
        }
    }

    flush(
        &mut data,
        &current_name,
        &current_version,
        current_resolution.take(),
        current_checksum.take(),
        std::mem::take(&mut current_deps),
    );
    Ok(data)
}
