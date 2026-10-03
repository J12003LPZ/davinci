//! Required capabilities for untrusted design execution. No permissive fallback.
use davinci_protocol::{MountAccess, MountRule, SandboxSpec};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn design_sandbox(
    private: &Path,
    node: &Path,
    package: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<SandboxSpec, String> {
    let cache = environment
        .get("PLAYWRIGHT_BROWSERS_PATH")
        .ok_or("design browser cache must be explicitly configured")?;
    let cache = Path::new(cache)
        .canonicalize()
        .map_err(|_| "design browser cache unavailable")?;
    let mut spec: SandboxSpec = serde_json::from_value(serde_json::json!({
        "id":format!("design-{}", uuid::Uuid::new_v4()),
        "mode":"workspace_write", "backend":"auto", "workspace":private,
        "network":{"mode":"denied"},
        "environment":{"allow":environment.keys().collect::<Vec<_>>()},
        "resources":{"timeout_ms":600000,"max_output_bytes":52428800},
        "process":{"allow_background":true,"max_background_lifetime_ms":600000},
        "required_capabilities":{
            "filesystem_isolation":true,"network_denied":true,"environment_isolation":true,
            "process_tree_isolation":true,"deterministic_teardown":true
        }
    }))
    .map_err(|_| "invalid design sandbox")?;
    let mut paths = vec![node.to_path_buf(), package.to_path_buf(), cache];
    if cfg!(target_os = "linux") {
        let config = Path::new(
            environment
                .get("FONTCONFIG_FILE")
                .ok_or("pinned font configuration required")?,
        );
        super::runtime::no_links(config).map_err(|e| e.to_string())?;
        let root = config.parent().ok_or("font runtime root unavailable")?;
        paths.push(config.to_path_buf());
        paths.push(root.join("fonts"));
    }
    #[cfg(unix)]
    for system in [
        "/usr",
        "/bin",
        "/lib",
        "/lib64",
        "/etc/fonts",
        "/System/Library",
        "/Library/Fonts",
    ] {
        if Path::new(system).exists() {
            paths.push(system.into());
        }
    }
    paths.sort();
    paths.dedup();
    for path in paths {
        let path = path.to_string_lossy().into_owned();
        spec.filesystem.mounts.push(MountRule {
            source: Some(path.clone()),
            target: path,
            access: MountAccess::ReadOnly,
        });
    }
    let private = private.to_string_lossy().into_owned();
    spec.filesystem.mounts.push(MountRule {
        source: Some(private.clone()),
        target: private,
        access: MountAccess::ReadWrite,
    });
    spec.validate().map_err(|e| e.to_string())?;
    Ok(spec)
}
