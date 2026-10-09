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
    /// Every distinct resolution per package name. `packages` keeps one
    /// primary record per name; a lockfile may legitimately install several
    /// versions of the same package for different parents.
    pub resolutions: BTreeMap<String, Vec<LockedPackage>>,
}

impl LockfileData {
    /// Record a resolved package, keeping distinct versions of one name.
    pub fn record_resolution(&mut self, package: &LockedPackage) {
        let versions = self.resolutions.entry(package.name.clone()).or_default();
        if !versions
            .iter()
            .any(|known| known.version == package.version && known.resolved == package.resolved)
        {
            versions.push(package.clone());
        }
    }

    /// Every resolution of every package, for dependency-reason queries.
    pub fn all_resolutions(&self) -> impl Iterator<Item = &LockedPackage> {
        self.resolutions.values().flatten()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(data: &LockfileData, name: &str) -> Vec<String> {
        let mut versions: Vec<_> = data.resolutions[name]
            .iter()
            .map(|package| package.version.clone())
            .collect();
        versions.sort();
        versions
    }

    #[test]
    fn every_lockfile_keeps_multiple_versions_of_one_package() {
        let npm = r#"{"lockfileVersion":3,"packages":{
            "":{},
            "node_modules/a":{"version":"1.0.0","dependencies":{"foo":"^1.0.0"}},
            "node_modules/foo":{"version":"1.0.0"},
            "node_modules/b":{"version":"1.0.0","dependencies":{"foo":"^2.0.0"}},
            "node_modules/b/node_modules/foo":{"version":"2.0.0"}}}"#;
        let pnpm = "lockfileVersion: '9.0'\n\npackages:\n\n  foo@1.0.0:\n    resolution: {integrity: sha512-a}\n\n  foo@2.0.0:\n    resolution: {integrity: sha512-b}\n";
        let yarn = "foo@^1.0.0:\n  version \"1.0.0\"\n\nfoo@^2.0.0:\n  version \"2.0.0\"\n";
        let berry = "__metadata:\n  version: 6\n\n\"foo@npm:^1.0.0\":\n  version: 1.0.0\n  resolution: \"foo@npm:1.0.0\"\n\n\"foo@npm:^2.0.0\":\n  version: 2.0.0\n  resolution: \"foo@npm:2.0.0\"\n";
        for (file, content) in [
            ("package-lock.json", npm),
            ("pnpm-lock.yaml", pnpm),
            ("yarn.lock", yarn),
            ("yarn.lock", berry),
        ] {
            let data = parse_lockfile(file, content).unwrap();
            assert_eq!(versions(&data, "foo"), ["1.0.0", "2.0.0"], "{file}");
        }
        let npm = parse_lockfile("package-lock.json", npm).unwrap();
        let parents: Vec<_> = npm
            .all_resolutions()
            .filter(|package| package.dependencies.contains_key("foo"))
            .map(|package| package.name.as_str())
            .collect();
        assert_eq!(parents, ["a", "b"]);
    }
}
