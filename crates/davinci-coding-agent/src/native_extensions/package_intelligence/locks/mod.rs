pub mod npm;
pub mod pnpm;
pub mod yarn;

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LockfileKind {
    #[default]
    Npm,
    Pnpm,
    YarnClassic,
    YarnBerry,
}

impl LockfileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::YarnClassic => "yarn-classic",
            Self::YarnBerry => "yarn-berry",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct LockfileData {
    pub kind: LockfileKind,
    pub lockfile_version: Option<String>,
    pub packages: BTreeMap<String, LockedPackage>,
    /// Map of workspace subpath (e.g. "packages/api") -> (package_name -> LockedPackage)
    pub workspace_packages: BTreeMap<String, BTreeMap<String, LockedPackage>>,
}

pub fn parse_lockfile(filename: &str, content: &str) -> Result<LockfileData, String> {
    if filename.ends_with("package-lock.json") || filename.ends_with("npm-shrinkwrap.json") {
        npm::parse(content)
    } else if filename.ends_with("pnpm-lock.yaml") {
        pnpm::parse(content)
    } else if filename.ends_with("yarn.lock") {
        yarn::parse(content)
    } else {
        Err(format!("unrecognized lockfile: {filename}"))
    }
}
