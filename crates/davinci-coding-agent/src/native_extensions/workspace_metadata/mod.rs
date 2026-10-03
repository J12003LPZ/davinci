//! Bounded package ownership and script facts shared by intelligence consumers.
mod manager;
use super::repo_intelligence::{RepoIndex, RepoIntelligence};
use davinci_agent::runtime::cache::digest;
pub use manager::PackageManager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub path: String,
    pub name: Option<String>,
    pub manifest: String,
    pub scripts: BTreeMap<String, String>,
    pub dependencies: BTreeSet<String>,
    pub declared_workspace: bool,
    pub package_manager: Option<PackageManager>,
    pub manager_evidence: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub packages: Vec<Package>,
    pub hashes: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub bytes_read: usize,
}

impl WorkspaceMetadata {
    pub fn discover(
        repo: &RepoIntelligence,
        index: &RepoIndex,
        authorize: &impl Fn(&str) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        let mut workspace_patterns = Vec::new();
        let mut managers = BTreeMap::new();
        for path in index.metadata.iter().take(2048) {
            authorize(path)?;
            let body = match repo.read_project_file(path, 256 * 1024) {
                Ok(body) => body,
                Err(_) => {
                    result
                        .warnings
                        .push(format!("metadata unavailable: {path}"));
                    continue;
                }
            };
            result.bytes_read += body.len();
            if result.bytes_read > 8 * 1024 * 1024 {
                result.warnings.push("metadata byte limit".into());
                break;
            }
            result.hashes.insert(path.clone(), digest(body.as_bytes()));
            if !path.ends_with("/package.json") && path != "package.json" {
                continue;
            }
            let value: Value = match serde_json::from_str(&body) {
                Ok(Value::Object(value)) => Value::Object(value),
                _ => {
                    result
                        .warnings
                        .push(format!("invalid package manifest: {path}"));
                    continue;
                }
            };
            if path == "package.json" {
                let patterns = value["workspaces"]
                    .as_array()
                    .or_else(|| value["workspaces"]["packages"].as_array());
                workspace_patterns = patterns
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                if value.get("workspaces").is_some() && patterns.is_none() {
                    result.warnings.push("invalid workspace declaration".into());
                }
            }
            let package_path = path
                .rsplit_once('/')
                .map_or(".", |(dir, _)| dir)
                .to_string();
            if let Some(value) = value.get("packageManager") {
                let manager = value.as_str().and_then(PackageManager::parse);
                if manager.is_none() {
                    result
                        .warnings
                        .push(format!("unsupported package manager in {path}"));
                }
                managers.insert(package_path.clone(), manager);
            }
            let scripts = value["scripts"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, value)| value.as_str().map(|s| (name.clone(), s.to_string())))
                .filter(|(name, value)| name.len() <= 128 && value.len() <= 4096)
                .collect();
            let dependencies = [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ]
            .iter()
            .flat_map(|field| {
                value[field]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(name, _)| name.clone())
            })
            .collect();
            result.packages.push(Package {
                path: package_path,
                name: value["name"].as_str().map(str::to_string),
                manifest: path.clone(),
                scripts,
                dependencies,
                declared_workspace: path == "package.json",
                package_manager: None,
                manager_evidence: String::new(),
            });
        }
        if index.metadata.len() > 2048 {
            result.warnings.push("metadata count limit".into());
        }
        let mut patterns = Vec::new();
        for pattern in workspace_patterns {
            let (exclude, pattern) = pattern
                .strip_prefix('!')
                .map_or((false, pattern.as_str()), |p| (true, p));
            match globset::GlobBuilder::new(pattern.trim_end_matches('/'))
                .literal_separator(true)
                .build()
            {
                Ok(glob) => patterns.push((exclude, glob.compile_matcher())),
                Err(_) => result.warnings.push("unsupported workspace pattern".into()),
            }
        }
        for package in &mut result.packages {
            package.declared_workspace |= patterns
                .iter()
                .any(|(exclude, p)| !exclude && p.is_match(&package.path))
                && !patterns
                    .iter()
                    .any(|(exclude, p)| *exclude && p.is_match(&package.path));
            let (manager, evidence) = manager::resolve(&package.path, &managers, &index.metadata);
            package.package_manager = manager;
            package.manager_evidence = evidence.into();
            if manager.is_none() {
                result
                    .warnings
                    .push(format!("package manager unresolved: {}", package.path));
            }
        }
        result
            .packages
            .sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.path.cmp(&b.path)));
        result.warnings.truncate(32);
        Ok(result)
    }

    pub fn owner(&self, file: &str) -> Option<&Package> {
        self.packages
            .iter()
            .find(|package| package.path == "." || file.starts_with(&format!("{}/", package.path)))
    }
}
