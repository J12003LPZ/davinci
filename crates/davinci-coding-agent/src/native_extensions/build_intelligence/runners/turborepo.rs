use crate::native_extensions::build_intelligence::model::BuildTarget;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct TurboConfig {
    #[serde(default)]
    pub tasks: BTreeMap<String, TurboTask>,
    #[serde(default)]
    pub pipeline: BTreeMap<String, TurboTask>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TurboTask {
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default = "default_cache")]
    pub cache: bool,
}

fn default_cache() -> bool {
    true
}

pub fn parse_turbo_json(root: &Path) -> Option<TurboConfig> {
    let turbo_path = root.join("turbo.json");
    if !turbo_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&turbo_path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn extract_turbo_targets(config: &TurboConfig, package_name: &str) -> Vec<BuildTarget> {
    let mut targets = Vec::new();
    let tasks = if !config.tasks.is_empty() {
        &config.tasks
    } else {
        &config.pipeline
    };

    for (task_name, task) in tasks {
        // Skip package-specific task overrides like "web#build" unless it matches our package
        if task_name.contains('#') {
            if let Some((pkg, t)) = task_name.split_once('#') {
                if pkg == package_name {
                    targets.push(BuildTarget {
                        target: t.to_string(),
                        package: package_name.to_string(),
                        runner: "turbo".to_string(),
                        executor: None,
                        command: Some(format!("turbo run {t}")),
                        inputs: task.inputs.clone(),
                        outputs: task.outputs.clone(),
                        depends_on: task.depends_on.clone(),
                        cacheable: task.cache,
                    });
                }
            }
        } else {
            targets.push(BuildTarget {
                target: task_name.clone(),
                package: package_name.to_string(),
                runner: "turbo".to_string(),
                executor: None,
                command: Some(format!("turbo run {task_name}")),
                inputs: task.inputs.clone(),
                outputs: task.outputs.clone(),
                depends_on: task.depends_on.clone(),
                cacheable: task.cache,
            });
        }
    }

    targets
}
