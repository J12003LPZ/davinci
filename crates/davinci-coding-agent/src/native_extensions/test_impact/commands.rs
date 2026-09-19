use super::{
    super::workspace_metadata::{Package, PackageManager, WorkspaceMetadata},
    analysis::RelatedTest,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_COMMANDS: usize = 32;
const MAX_FILTERS: usize = 64;
const MAX_ARG_BYTES: usize = 8192;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCommand {
    pub cwd: String,
    pub program: String,
    pub argv: Vec<String>,
    pub framework: String,
    pub reason: String,
    pub manager_evidence: String,
    pub requires_authorization: bool,
}

pub fn plans(
    tests: &[RelatedTest],
    metadata: &WorkspaceMetadata,
    changed: &[String],
    incomplete: bool,
) -> (
    Vec<VerificationCommand>,
    Vec<VerificationCommand>,
    Vec<String>,
) {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    for test in tests {
        groups
            .entry(test.package.clone())
            .or_default()
            .insert(test.path.clone());
    }
    let mut affected: BTreeSet<_> = groups.keys().cloned().collect();
    affected.extend(
        changed
            .iter()
            .filter_map(|path| metadata.owner(path))
            .map(|p| p.path.clone()),
    );
    if changed
        .iter()
        .any(|path| metadata.owner(path).is_some_and(|p| p.path == "."))
    {
        affected.extend(
            metadata
                .packages
                .iter()
                .filter(|p| p.declared_workspace)
                .map(|p| p.path.clone()),
        );
    }
    if incomplete {
        affected.extend(metadata.packages.iter().map(|p| p.path.clone()));
    }
    let mut first = Vec::new();
    let mut broader = Vec::new();
    let mut warnings = Vec::new();
    for package in &metadata.packages {
        if !affected.contains(&package.path) {
            continue;
        }
        let Some(script) = package.scripts.get("test") else {
            warnings.push(format!(
                "no declared test script for {}; choose broader verification",
                package.path
            ));
            continue;
        };
        let Some(manager) = package.package_manager else {
            continue;
        };
        if broader.len() == MAX_COMMANDS {
            warnings.push(format!(
                "command limit: {} affected packages; remaining package checks must be planned",
                affected.len()
            ));
            break;
        }
        broader.push(package_command(
            package,
            manager,
            "package",
            "required package verification",
        ));
        let Some(paths) = groups.get(&package.path) else {
            continue;
        };
        let relative: Vec<String> = paths
            .iter()
            .map(|path| {
                let path = if package.path == "." {
                    path.as_str()
                } else {
                    path.strip_prefix(&format!("{}/", package.path))
                        .unwrap_or(path)
                };
                if path.starts_with('-') {
                    format!("./{path}")
                } else {
                    path.to_string()
                }
            })
            .collect();
        if relative.len() > MAX_FILTERS
            || relative.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES
        {
            first.push(package_command(
                package,
                manager,
                "package",
                "filter limit; complete package test command",
            ));
            warnings.push(format!(
                "targeted argument limit in {}; using complete package tests",
                package.path
            ));
            continue;
        }
        let words: Vec<_> = script.split_whitespace().collect();
        let plain = !script.chars().any(|c| {
            c.is_control() || matches!(c, ';' | '&' | '|' | '$' | '\x60' | '>' | '<' | '"' | '\'')
        });
        if plain
            && words.starts_with(&["node", "--test"])
            && words.iter().skip(2).all(|word| !word.starts_with('-'))
        {
            first.push(VerificationCommand {
                cwd: package.path.clone(),
                program: "node".into(),
                argv: std::iter::once("--test".into()).chain(relative).collect(),
                framework: "node:test".into(),
                reason: "explicit selected test paths; package lifecycle remains in broader tier"
                    .into(),
                manager_evidence: package.manager_evidence.clone(),
                requires_authorization: true,
            });
        } else if plain
            && matches!(
                words.as_slice(),
                ["vitest"] | ["vitest", "run"] | ["jest"] | ["playwright", "test"]
            )
        {
            let framework = words[0];
            let mut command =
                package_command(package, manager, framework, "selected framework test paths");
            if manager == PackageManager::Npm {
                command.argv.push("--".into());
            }
            if framework == "vitest" && words.len() == 1 {
                command.argv.push("--run".into());
            }
            if framework == "jest" {
                command.argv.extend(
                    ["--runTestsByPath", "--watch=false", "--watchAll=false"].map(str::to_string),
                );
            }
            if framework == "playwright" {
                command
                    .argv
                    .extend(relative.into_iter().map(|p| regex::escape(&p)));
            } else {
                command.argv.extend(relative);
            }
            first.push(command);
        } else {
            first.push(package_command(
                package,
                manager,
                "custom",
                "unknown filter semantics; run declared package test script",
            ));
            warnings.push(format!(
                "cannot safely narrow custom test script in {}",
                package.path
            ));
        }
    }
    if broader.is_empty() {
        warnings.push("no executable test command discovered; verification is unresolved".into());
    }
    (first, broader, warnings)
}

fn package_command(
    package: &Package,
    manager: PackageManager,
    framework: &str,
    reason: &str,
) -> VerificationCommand {
    VerificationCommand {
        cwd: package.path.clone(),
        program: manager.program().into(),
        argv: vec!["run".into(), "test".into()],
        framework: framework.into(),
        reason: reason.into(),
        manager_evidence: package.manager_evidence.clone(),
        requires_authorization: true,
    }
}
