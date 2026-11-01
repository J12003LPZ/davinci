use super::{
    assets::{trusted_manifest, HostAssets},
    NodeCodeModeHost,
};
use davinci_agent::codemode::CodeModeMode;
use std::path::{Path, PathBuf};

/// The controlled-mode release gate. It flips only after the S05/S15 recovery
/// matrix passes on every supported platform; until then `controlled` parses
/// and reports why it is unavailable instead of silently degrading to
/// read-only.
pub const CONTROLLED_RECOVERY_ACCEPTED: bool = true;

/// Owner approval to supersede the 2026-10-04 C08 resident-service prohibition
/// for detached Script runs, granted 2026-11-01. Detached runs still require
/// controlled mode, the durable run store and host admission.
pub const BACKGROUND_SCRIPTS_APPROVED: bool = true;

#[derive(Debug, PartialEq, Eq)]
pub struct ReadOnlyConfig {
    pub node_path: PathBuf,
    pub host_path: PathBuf,
    pub mode: CodeModeMode,
}

/// Where `/config` expects the Codemode runtime when settings name no paths:
/// `<agent dir>/codemode/node` (the admitted Node build) and
/// `<agent dir>/codemode/host` (the runtime bundle).
pub fn managed_paths(agent_dir: &Path) -> ReadOnlyConfig {
    let root = agent_dir.join("codemode");
    let node = if cfg!(windows) {
        root.join("node").join("node.exe")
    } else {
        root.join("node").join("bin").join("node")
    };
    ReadOnlyConfig {
        node_path: node,
        host_path: root.join("host"),
        mode: CodeModeMode::ReadOnly,
    }
}

/// The mode `--codemode` or `/config` selects, before the runtime is admitted.
pub fn configured_mode<'a>(cli: Option<&'a str>, user: Option<&'a serde_json::Value>) -> &'a str {
    cli.or_else(|| {
        user.and_then(|value| value.get("mode"))
            .and_then(|value| value.as_str())
    })
    .unwrap_or("off")
}

pub fn resolve_config(
    cli: Option<&str>,
    user: Option<&serde_json::Value>,
    agent_dir: &Path,
) -> Result<Option<ReadOnlyConfig>, String> {
    let mode = match configured_mode(cli, user) {
        "off" => return Ok(None),
        "controlled" => {
            if !CONTROLLED_RECOVERY_ACCEPTED {
                return Err(
                    "Controlled Codemode is unavailable until its recovery acceptance gates pass"
                        .into(),
                );
            }
            CodeModeMode::Controlled
        }
        "read-only" => CodeModeMode::ReadOnly,
        _ => return Err("Invalid Codemode mode".into()),
    };
    let managed = managed_paths(agent_dir);
    let path = |key, default: PathBuf| -> Result<PathBuf, String> {
        let Some(value) = user.and_then(|value| value.get(key)) else {
            return Ok(default);
        };
        let path = value
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| format!("Codemode {key} must be a path"))?;
        if !path.is_absolute() {
            return Err(format!("Codemode {key} must be absolute"));
        }
        Ok(path)
    };
    Ok(Some(ReadOnlyConfig {
        node_path: path("nodePath", managed.node_path)?,
        host_path: path("hostPath", managed.host_path)?,
        mode,
    }))
}

impl ReadOnlyConfig {
    /// A runtime inside the workspace is refused as project-provided, unless it
    /// sits in DaVinci's own `<agent_dir>/codemode`. Running from the home
    /// directory puts the agent dir inside the workspace; the write tools
    /// already treat it as protected.
    pub fn admit(&self, workspace: &Path, agent_dir: &Path) -> Result<NodeCodeModeHost, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| "Codemode workspace unavailable")?;
        let node = self.node_path.canonicalize().map_err(|_| {
            format!(
                "Codemode Node runtime unavailable: no Node 24.21.0 at {}",
                self.node_path.display()
            )
        })?;
        let managed_root = agent_dir.join("codemode");
        let trusted = managed_root.canonicalize().ok();
        if node.starts_with(&workspace)
            && !trusted
                .as_ref()
                .is_some_and(|trusted| node.starts_with(trusted))
        {
            return Err(format!(
                "Workspace-provided Node runtime is not admitted: {} is inside the workspace {}",
                node.display(),
                workspace.display()
            ));
        }
        let manifest = trusted_manifest()
            .map_err(|error| format!("Codemode manifest unavailable: {error}"))?;
        let assets = HostAssets::validate_installation(
            &self.host_path,
            &workspace,
            Some(&managed_root),
            &manifest,
        )
        .map_err(|error| {
            format!(
                "Codemode host admission failed at {}: {error}",
                self.host_path.display()
            )
        })?;
        NodeCodeModeHost::new(&self.node_path, assets)
            .map(|host| {
                host.with_controlled_mode(matches!(self.mode, CodeModeMode::Controlled))
                    .with_background_scripts(
                        BACKGROUND_SCRIPTS_APPROVED
                            && matches!(self.mode, CodeModeMode::Controlled),
                    )
            })
            .map_err(|error| {
                format!(
                    "Codemode runtime admission failed at {}: {error}",
                    self.node_path.display()
                )
            })
    }
}
