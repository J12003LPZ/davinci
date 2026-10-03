use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value.split('@').next()? {
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }
    fn lockfile(name: &str) -> Option<Self> {
        match name {
            "package-lock.json" | "npm-shrinkwrap.json" => Some(Self::Npm),
            "pnpm-lock.yaml" | "pnpm-workspace.yaml" => Some(Self::Pnpm),
            "yarn.lock" => Some(Self::Yarn),
            "bun.lock" | "bun.lockb" => Some(Self::Bun),
            _ => None,
        }
    }
}

pub fn resolve(
    path: &str,
    declarations: &BTreeMap<String, Option<PackageManager>>,
    metadata: &[String],
) -> (Option<PackageManager>, &'static str) {
    let mut directory = path;
    loop {
        if let Some(manager) = declarations.get(directory) {
            return (*manager, "packageManager field");
        }
        let locks: BTreeSet<_> = metadata
            .iter()
            .filter_map(|path| {
                let (parent, name) = path.rsplit_once('/').unwrap_or((".", path));
                (parent == directory)
                    .then(|| PackageManager::lockfile(name))
                    .flatten()
            })
            .collect();
        if locks.len() == 1 {
            return (locks.first().copied(), "workspace lockfile");
        }
        if locks.len() > 1 {
            return (None, "conflicting package manager lockfiles");
        }
        if directory == "." {
            break;
        }
        directory = directory.rsplit_once('/').map_or(".", |(parent, _)| parent);
    }
    (
        Some(PackageManager::Npm),
        "npm convention; no explicit package manager",
    )
}
