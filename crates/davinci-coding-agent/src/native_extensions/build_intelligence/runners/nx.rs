use crate::native_extensions::build_intelligence::model::BuildTarget;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct NxConfig {
    #[serde(default, rename = "targetDefaults")]
    pub target_defaults: BTreeMap<String, NxTargetDefault>,
    #[serde(default, rename = "namedInputs")]
    pub named_inputs: BTreeMap<String, serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct NxTargetDefault {
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<serde_json::Value>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default = "default_cache")]
    pub cache: bool,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct NxProjectConfig {
    pub name: Option<String>,
    #[serde(default)]
    pub targets: BTreeMap<String, NxProjectTarget>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NxProjectTarget {
    pub executor: Option<String>,
    pub command: Option<String>,
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default = "default_cache")]
    pub cache: bool,
}

fn default_cache() -> bool {
    true
}

pub fn parse_nx_json(root: &Path) -> Option<NxConfig> {
    let nx_path = root.join("nx.json");
    if !nx_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&nx_path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn parse_project_json(package_dir: &Path) -> Option<NxProjectConfig> {
    let project_path = package_dir.join("project.json");
    if !project_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&project_path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn extract_nx_targets(
    nx_config: Option<&NxConfig>,
    project_config: Option<&NxProjectConfig>,
    package_name: &str,
) -> Vec<BuildTarget> {
    let mut targets = Vec::new();

    // Project-specific targets from project.json
    if let Some(pc) = project_config {
        for (target_name, target) in &pc.targets {
            let mut depends_on = target.depends_on.clone();
            let mut outputs = target.outputs.clone();
            let mut cacheable = target.cache;

            // Inherit from target_defaults in nx.json if not explicitly specified
            if let Some(nc) = nx_config {
                if let Some(def) = nc.target_defaults.get(target_name) {
                    if depends_on.is_empty() {
                        depends_on = def.depends_on.clone();
                    }
                    if outputs.is_empty() {
                        outputs = def.outputs.clone();
                    }
                    if !cacheable && def.cache {
                        cacheable = true;
                    }
                }
            }

            let command = target.command.clone().or_else(|| {
                target
                    .executor
                    .as_ref()
                    .map(|exec| format!("nx run {package_name}:{target_name} ({exec})"))
            });

            targets.push(BuildTarget {
                target: target_name.clone(),
                package: package_name.to_string(),
                runner: "nx".to_string(),
                executor: target.executor.clone(),
                command,
                inputs: Vec::new(),
                outputs,
                depends_on,
                cacheable,
            });
        }
    } else if let Some(nc) = nx_config {
        // Fallback to nx.json target defaults if project.json not present
        for (target_name, def) in &nc.target_defaults {
            targets.push(BuildTarget {
                target: target_name.clone(),
                package: package_name.to_string(),
                runner: "nx".to_string(),
                executor: None,
                command: Some(format!("nx run {package_name}:{target_name}")),
                inputs: Vec::new(),
                outputs: def.outputs.clone(),
                depends_on: def.depends_on.clone(),
                cacheable: def.cache,
            });
        }
    }

    targets
}
