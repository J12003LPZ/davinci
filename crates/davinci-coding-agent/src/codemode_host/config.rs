use super::{
    assets::{trusted_manifest, HostAssets},
    NodeCodeModeHost,
};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct ReadOnlyConfig {
    pub node_path: PathBuf,
    pub host_path: PathBuf,
}

pub fn resolve_config(
    cli: Option<&str>,
    user: Option<&serde_json::Value>,
) -> Result<Option<ReadOnlyConfig>, String> {
    let mode = cli
        .or_else(|| {
            user.and_then(|value| value.get("mode"))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("off");
    match mode {
        "off" => return Ok(None),
        "controlled" => {
            return Err(
                "Controlled Codemode is unavailable until its recovery acceptance gates pass"
                    .into(),
            )
        }
        "read-only" => {}
        _ => return Err("Invalid Codemode mode".into()),
    }
    let path = |key| -> Result<PathBuf, String> {
        let path = user
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .map(PathBuf::from)
            .ok_or_else(|| format!("Codemode read-only requires user {key}"))?;
        if !path.is_absolute() {
            return Err(format!("Codemode {key} must be absolute"));
        }
        Ok(path)
    };
    Ok(Some(ReadOnlyConfig {
        node_path: path("nodePath")?,
        host_path: path("hostPath")?,
    }))
}

impl ReadOnlyConfig {
    pub fn admit(&self, workspace: &Path) -> Result<NodeCodeModeHost, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| "Codemode workspace unavailable")?;
        let node = self
            .node_path
            .canonicalize()
            .map_err(|_| "Codemode Node runtime unavailable")?;
        if node.starts_with(&workspace) {
            return Err("Workspace-provided Node runtime is not admitted".into());
        }
        let manifest = trusted_manifest()
            .map_err(|error| format!("Codemode manifest unavailable: {error}"))?;
        let assets = HostAssets::validate_installation(&self.host_path, &workspace, &manifest)
            .map_err(|error| format!("Codemode host admission failed: {error}"))?;
        NodeCodeModeHost::new(&self.node_path, assets)
            .map_err(|error| format!("Codemode runtime admission failed: {error}"))
    }
}
