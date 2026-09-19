use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIntelligenceConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for BuildIntelligenceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePackage {
    pub name: String,
    pub relative_path: String,
    pub manifest_path: String,
    pub package_manager: String,
    pub scripts: BTreeMap<String, String>,
    pub dependencies: Vec<String>,
    pub dev_dependencies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tsconfig_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePackagesResult {
    pub packages: Vec<WorkspacePackage>,
    pub package_manager: String,
    pub is_monorepo: bool,
    pub task_runner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildTarget {
    pub target: String,
    pub package: String,
    pub runner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub cacheable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildTargetsResult {
    pub targets: Vec<BuildTarget>,
    pub task_runner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildDependenciesResult {
    pub package: String,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    pub project_references: Vec<String>,
    pub pipeline_dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildAffectedResult {
    pub changed_inputs: Vec<String>,
    pub directly_affected: Vec<String>,
    pub transitive_affected: Vec<String>,
    pub all_affected_packages: Vec<String>,
    #[serde(default)]
    pub affected_packages: Vec<String>,
    pub affected_targets: Vec<BuildTarget>,
    pub reverse_dependency_paths: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCommandResult {
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub runner: String,
    pub targets: Vec<String>,
    pub cache_hint: String,
    pub raw_command: String,
    pub command: String,
    pub supports_cache: bool,
}
