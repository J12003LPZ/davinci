use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Extension manifest discovery matching `vendor/pi/packages/coding-agent/src/core/extensions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<ExtensionTool>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub command: Option<String>,
}

pub fn discover_extensions(agent_dir: &Path, names: &[String]) -> Vec<ExtensionManifest> {
    let root = agent_dir.join("extensions");
    let mut found = Vec::new();
    for name in names {
        let dir = root.join(name);
        let manifest_path = if dir.join("pi.extension.json").exists() {
            dir.join("pi.extension.json")
        } else {
            dir.join("package.json")
        };
        if let Ok(raw) = fs::read_to_string(&manifest_path) {
            if let Ok(mut manifest) = serde_json::from_str::<ExtensionManifest>(&raw) {
                manifest.path = Some(dir.display().to_string());
                found.push(manifest);
                continue;
            }
        }
        found.push(ExtensionManifest {
            name: name.clone(),
            description: String::new(),
            tools: Vec::new(),
            path: Some(name.clone()),
        });
    }
    found
}

pub fn extension_tool_names(manifests: &[ExtensionManifest]) -> Vec<String> {
    manifests
        .iter()
        .flat_map(|manifest| manifest.tools.iter().map(|tool| tool.name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_pi_extension_json() {
        let dir = tempdir().unwrap();
        let ext = dir.path().join("extensions").join("demo");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("pi.extension.json"),
            r#"{"name":"demo","tools":[{"name":"ticket","description":"lookup"}]}"#,
        )
        .unwrap();
        let found = discover_extensions(dir.path(), &["demo".into()]);
        assert_eq!(found[0].name, "demo");
        assert_eq!(extension_tool_names(&found), ["ticket"]);
    }
}
