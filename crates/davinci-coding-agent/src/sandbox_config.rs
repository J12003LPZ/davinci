use davinci_protocol::{
    ContainerPolicy, ContainerRuntime, EnvironmentPolicy, FilesystemPolicy, MountAccess, MountRule,
    NetworkPolicy, ProcessPolicy, ResourcePolicy, SandboxBackendKind, SandboxCapabilities, SandboxId,
    SandboxMode, SandboxSpec,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxSettings {
    pub mode: Option<String>,
    pub backend: Option<String>,
    pub container: Option<SandboxContainerSettings>,
    pub network: Option<SandboxNetworkSettings>,
    pub resources: Option<SandboxResourceSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxContainerSettings {
    pub runtime: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxNetworkSettings {
    pub mode: Option<String>,
    pub domains: Vec<String>,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxResourceSettings {
    pub timeout_seconds: Option<u64>,
    pub max_memory_mb: Option<u64>,
    pub max_processes: Option<u32>,
}

pub fn resolve_sandbox_settings(
    workspace: &Path,
    global: Option<&SandboxSettings>,
    project: Option<&SandboxSettings>,
    project_trusted: bool,
) -> Result<Option<SandboxSpec>, String> {
    let Some(global) = global else {
        // Compatibility is explicit: absence of user/global sandbox settings
        // means no OS sandbox is claimed. A project cannot opt the user into
        // broader execution or silently change this security boundary.
        return Ok(None);
    };
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("sandbox workspace unavailable: {error}"))?;
    let mut effective = global.clone();

    if project_trusted {
        if let Some(project) = project {
            narrow_settings(&mut effective, project)?;
        }
    }

    let mode = parse_mode(effective.mode.as_deref().unwrap_or("workspace_write"))?;
    let backend = parse_backend(effective.backend.as_deref().unwrap_or("auto"))?;
    if mode != SandboxMode::FullAccess && backend == SandboxBackendKind::Host {
        return Err("host sandbox backend requires explicit full_access mode".into());
    }
    if mode == SandboxMode::FullAccess && backend != SandboxBackendKind::Host && backend != SandboxBackendKind::Auto {
        return Err("full_access may use only host or auto backend".into());
    }
    let container = parse_container(effective.container.as_ref(), backend)?;

    let network = parse_network(effective.network.as_ref(), mode)?;
    if mode == SandboxMode::FullAccess && !matches!(network, NetworkPolicy::Unrestricted) {
        return Err("full_access host execution cannot claim network isolation".into());
    }

    let resources = parse_resources(effective.resources.as_ref())?;
    let workspace_text = workspace
        .to_str()
        .ok_or("sandbox workspace path is not UTF-8")?
        .to_string();
    let mut mounts = runtime_mounts(&workspace);
    match mode {
        SandboxMode::Restricted => mounts.push(MountRule {
            source: Some(workspace_text.clone()),
            target: workspace_text.clone(),
            access: MountAccess::ReadOnly,
        }),
        SandboxMode::WorkspaceWrite => mounts.push(MountRule {
            source: Some(workspace_text.clone()),
            target: workspace_text.clone(),
            access: MountAccess::ReadWrite,
        }),
        SandboxMode::NoExecution | SandboxMode::FullAccess => {}
    }
    if !matches!(mode, SandboxMode::NoExecution | SandboxMode::FullAccess) {
        mounts.push(MountRule {
            source: None,
            target: "/tmp/davinci-home".into(),
            access: MountAccess::Temporary,
        });
    }

    let mut environment = EnvironmentPolicy::default();
    if !matches!(mode, SandboxMode::NoExecution | SandboxMode::FullAccess) {
        environment
            .inject
            .insert("HOME".into(), "/tmp/davinci-home".into());
        environment.inject.insert("TMPDIR".into(), "/tmp".into());
        environment.inject.insert("TMP".into(), "/tmp".into());
        environment.inject.insert("TEMP".into(), "/tmp".into());
    }

    let mut required = SandboxCapabilities {
        environment_isolation: mode != SandboxMode::NoExecution,
        process_tree_isolation: mode != SandboxMode::NoExecution,
        output_limit: mode != SandboxMode::NoExecution,
        timeout: mode != SandboxMode::NoExecution,
        deterministic_teardown: mode != SandboxMode::NoExecution,
        ..Default::default()
    };
    if matches!(mode, SandboxMode::Restricted | SandboxMode::WorkspaceWrite) {
        required.filesystem_isolation = true;
        required.ephemeral_temp = true;
        if matches!(network, NetworkPolicy::Denied) {
            required.network_denied = true;
        }
        if matches!(network, NetworkPolicy::AllowList { .. }) {
            required.network_allowlist = true;
        }
    }
    if resources.max_memory_bytes.is_some() {
        required.memory_limit = true;
    }
    if resources.max_processes.is_some() {
        required.pid_limit = true;
    }

    Ok(Some(SandboxSpec {
        id: SandboxId(format!("session-{}", uuid::Uuid::new_v4())),
        mode,
        backend,
        container,
        workspace: workspace_text,
        filesystem: FilesystemPolicy { mounts },
        network,
        environment,
        resources,
        process: ProcessPolicy {
            // Managed processes remain available, but ownership and teardown
            // stay with the existing session supervisor. The manager can apply
            // the lifetime bound independently of foreground command timeout.
            allow_background: mode != SandboxMode::NoExecution,
            max_background_lifetime_ms: if mode == SandboxMode::FullAccess {
                None
            } else {
                Some(30 * 60 * 1000)
            },
        },
        required_capabilities: required,
    }))
}

fn narrow_settings(
    global: &mut SandboxSettings,
    project: &SandboxSettings,
) -> Result<(), String> {
    let global_mode = parse_mode(global.mode.as_deref().unwrap_or("workspace_write"))?;
    if let Some(value) = project.mode.as_deref() {
        let project_mode = parse_mode(value)?;
        if !project_mode.is_no_more_permissive_than(global_mode) {
            return Err("project sandbox mode cannot widen global authority".into());
        }
        global.mode = Some(value.to_string());
    }

    if let Some(value) = project.backend.as_deref() {
        let requested = parse_backend(value)?;
        let configured = parse_backend(global.backend.as_deref().unwrap_or("auto"))?;
        let allowed = requested == configured
            || (configured == SandboxBackendKind::Auto && requested != SandboxBackendKind::Host);
        if !allowed {
            return Err("project sandbox backend cannot weaken global isolation".into());
        }
        global.backend = Some(value.to_string());
    }

    if let Some(project_container) = project.container.as_ref() {
        let Some(global_container) = global.container.as_ref() else {
            return Err("project sandbox cannot introduce container runtime/image authority".into());
        };
        if project_container != global_container {
            return Err("project sandbox cannot change container runtime or image".into());
        }
    }

    if let Some(project_network) = project.network.as_ref() {
        let global_network = parse_network(global.network.as_ref(), global_mode)?;
        let project_network_policy = parse_network(Some(project_network), global_mode)?;
        if !network_is_no_more_permissive(&global_network, &project_network_policy) {
            return Err("project sandbox network cannot widen global authority".into());
        }
        global.network = Some(project_network.clone());
    }

    if let Some(project_resources) = project.resources.as_ref() {
        let global_resources = parse_resources(global.resources.as_ref())?;
        let project_resources_parsed = parse_resources(Some(project_resources))?;
        ensure_limit_not_weaker(
            global_resources.timeout_ms,
            project_resources_parsed.timeout_ms,
            "timeout",
        )?;
        ensure_limit_not_weaker(
            global_resources.max_memory_bytes,
            project_resources_parsed.max_memory_bytes,
            "memory",
        )?;
        ensure_limit_not_weaker_u32(
            global_resources.max_processes,
            project_resources_parsed.max_processes,
            "process",
        )?;
        global.resources = Some(project_resources.clone());
    }
    Ok(())
}

fn parse_mode(value: &str) -> Result<SandboxMode, String> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "none" | "no_execution" | "disabled" => Ok(SandboxMode::NoExecution),
        "restricted" | "read_only" => Ok(SandboxMode::Restricted),
        "workspace_write" | "workspace" => Ok(SandboxMode::WorkspaceWrite),
        "full_access" | "full" => Ok(SandboxMode::FullAccess),
        other => Err(format!("unknown sandbox mode: {other}")),
    }
}

fn parse_backend(value: &str) -> Result<SandboxBackendKind, String> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "auto" => Ok(SandboxBackendKind::Auto),
        "linux_bubblewrap" | "bubblewrap" | "bwrap" => Ok(SandboxBackendKind::LinuxBubblewrap),
        "container" | "docker" | "podman" => Ok(SandboxBackendKind::Container),
        "host" => Ok(SandboxBackendKind::Host),
        other => Err(format!("unknown sandbox backend: {other}")),
    }
}

fn parse_container(
    settings: Option<&SandboxContainerSettings>,
    backend: SandboxBackendKind,
) -> Result<Option<ContainerPolicy>, String> {
    let Some(settings) = settings else {
        if backend == SandboxBackendKind::Container {
            return Err("container sandbox backend requires sandbox.container.image".into());
        }
        return Ok(None);
    };
    let image = settings
        .image
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("sandbox.container.image is required when container settings are present")?;
    let runtime = match settings
        .runtime
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "auto" => ContainerRuntime::Auto,
        "docker" => ContainerRuntime::Docker,
        "podman" => ContainerRuntime::Podman,
        other => return Err(format!("unknown container runtime: {other}")),
    };
    Ok(Some(ContainerPolicy {
        runtime,
        image: image.to_string(),
    }))
}


fn parse_network(
    settings: Option<&SandboxNetworkSettings>,
    mode: SandboxMode,
) -> Result<NetworkPolicy, String> {
    let default = if mode == SandboxMode::FullAccess {
        "unrestricted"
    } else {
        "deny"
    };
    let mode = settings
        .and_then(|settings| settings.mode.as_deref())
        .unwrap_or(default)
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    match mode.as_str() {
        "deny" | "denied" | "none" => Ok(NetworkPolicy::Denied),
        "unrestricted" | "allow" => Ok(NetworkPolicy::Unrestricted),
        "allowlist" | "allow_list" => {
            let settings = settings.ok_or("network allowlist requires settings")?;
            if settings.domains.is_empty() {
                return Err("network allowlist requires at least one domain".into());
            }
            Ok(NetworkPolicy::AllowList {
                domains: settings.domains.clone(),
                ports: settings.ports.clone(),
            })
        }
        other => Err(format!("unknown sandbox network mode: {other}")),
    }
}

fn parse_resources(
    settings: Option<&SandboxResourceSettings>,
) -> Result<ResourcePolicy, String> {
    let Some(settings) = settings else {
        return Ok(ResourcePolicy {
            timeout_ms: Some(300_000),
            max_output_bytes: Some(4 * 1024 * 1024),
            ..Default::default()
        });
    };
    let timeout_ms = settings
        .timeout_seconds
        .map(|seconds| seconds.checked_mul(1000).ok_or("sandbox timeout overflows"))
        .transpose()?
        .or(Some(300_000));
    let max_memory_bytes = settings
        .max_memory_mb
        .map(|mb| mb.checked_mul(1024 * 1024).ok_or("sandbox memory limit overflows"))
        .transpose()?;
    if timeout_ms == Some(0)
        || max_memory_bytes == Some(0)
        || settings.max_processes == Some(0)
    {
        return Err("sandbox resource limits must be greater than zero".into());
    }
    Ok(ResourcePolicy {
        timeout_ms,
        max_memory_bytes,
        max_processes: settings.max_processes,
        max_output_bytes: Some(4 * 1024 * 1024),
        ..Default::default()
    })
}

fn runtime_mounts(workspace: &Path) -> Vec<MountRule> {
    let mut sources = Vec::<PathBuf>::new();
    #[cfg(unix)]
    {
        for path in [
            "/usr",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/etc/ssl",
            "/etc/ca-certificates",
            "/etc/passwd",
            "/etc/group",
            "/etc/ld.so.cache",
            "/etc/ld.so.conf",
            "/etc/ld.so.conf.d",
        ] {
            push_runtime_source(&mut sources, Path::new(path), workspace);
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                push_runtime_source(&mut sources, &directory, workspace);
            }
        }
        if let Some(home) = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| {
            std::env::var_os("HOME").map(PathBuf::from).map(|home| home.join(".rustup"))
        }) {
            push_runtime_source(&mut sources, &home, workspace);
        }
        if let Some(cargo) = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
            std::env::var_os("HOME").map(PathBuf::from).map(|home| home.join(".cargo"))
        }) {
            for relative in ["bin", "registry", "git"] {
                push_runtime_source(&mut sources, &cargo.join(relative), workspace);
            }
        }
    }
    sources
        .into_iter()
        .filter_map(|source| {
            let target = source.to_str()?.to_string();
            Some(MountRule {
                source: Some(target.clone()),
                target,
                access: MountAccess::ReadOnly,
            })
        })
        .collect()
}

fn push_runtime_source(sources: &mut Vec<PathBuf>, path: &Path, workspace: &Path) {
    let Ok(path) = path.canonicalize() else {
        return;
    };
    if path.starts_with(workspace)
        || exposes_user_home_root(&path)
        || sources.iter().any(|seen| seen == &path)
    {
        return;
    }
    // Avoid redundant nested mounts when an already declared parent covers it.
    if sources.iter().any(|seen| path.starts_with(seen)) {
        return;
    }
    sources.retain(|seen| !seen.starts_with(&path));
    sources.push(path);
}

fn exposes_user_home_root(path: &Path) -> bool {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let Ok(home) = home.canonicalize() else {
        return false;
    };
    // Mounting HOME itself, or one of its ancestors (for example /home or /),
    // would make credential descendants such as ~/.ssh and ~/.aws readable.
    home.starts_with(path)
}

fn network_is_no_more_permissive(parent: &NetworkPolicy, child: &NetworkPolicy) -> bool {
    match (parent, child) {
        (NetworkPolicy::Denied, NetworkPolicy::Denied) => true,
        (NetworkPolicy::AllowList { .. }, NetworkPolicy::Denied) => true,
        (
            NetworkPolicy::AllowList {
                domains: parent_domains,
                ports: parent_ports,
            },
            NetworkPolicy::AllowList {
                domains: child_domains,
                ports: child_ports,
            },
        ) => {
            child_domains.iter().all(|domain| parent_domains.contains(domain))
                && child_ports.iter().all(|port| parent_ports.contains(port))
        }
        (NetworkPolicy::Unrestricted, _) => true,
        _ => false,
    }
}

fn ensure_limit_not_weaker(
    parent: Option<u64>,
    child: Option<u64>,
    name: &str,
) -> Result<(), String> {
    if let Some(parent) = parent {
        if child.is_none_or(|child| child > parent) {
            return Err(format!("project sandbox {name} limit cannot widen global authority"));
        }
    }
    Ok(())
}

fn ensure_limit_not_weaker_u32(
    parent: Option<u32>,
    child: Option<u32>,
    name: &str,
) -> Result<(), String> {
    if let Some(parent) = parent {
        if child.is_none_or(|child| child > parent) {
            return Err(format!("project sandbox {name} limit cannot widen global authority"));
        }
    }
    Ok(())
}


pub fn format_sandbox_status(spec: Option<&SandboxSpec>) -> String {
    let Some(spec) = spec else {
        return [
            "Sandbox: disabled (compatibility mode)",
            "Enforcement: none claimed",
            "Permission policy remains separate from OS isolation",
        ]
        .join("\n");
    };

    let mode = match spec.mode {
        SandboxMode::NoExecution => "no_execution",
        SandboxMode::Restricted => "restricted",
        SandboxMode::WorkspaceWrite => "workspace_write",
        SandboxMode::FullAccess => "full_access",
    };
    let backend = match spec.backend {
        SandboxBackendKind::Auto => "auto",
        SandboxBackendKind::LinuxBubblewrap => "linux_bubblewrap",
        SandboxBackendKind::Container => "container",
        SandboxBackendKind::Host => "host",
    };
    let network = match &spec.network {
        NetworkPolicy::Denied => "denied".to_string(),
        NetworkPolicy::Unrestricted => "unrestricted".to_string(),
        NetworkPolicy::AllowList { domains, ports } => format!(
            "allowlist ({} domains, {} ports; requires a backend that enforces DNS + egress)",
            domains.len(),
            ports.len()
        ),
    };
    let workspace_access = spec
        .filesystem
        .mounts
        .iter()
        .find(|mount| {
            mount.source.as_deref() == Some(spec.workspace.as_str())
                && mount.target == spec.workspace
        })
        .map(|mount| match mount.access {
            MountAccess::ReadOnly => "read-only",
            MountAccess::ReadWrite => "read-write",
            MountAccess::Temporary => "temporary",
            MountAccess::Hidden => "hidden",
        })
        .unwrap_or("not mounted");
    let memory = spec
        .resources
        .max_memory_bytes
        .map(|bytes| format!("{} MiB", bytes / (1024 * 1024)))
        .unwrap_or_else(|| "not required".into());
    let processes = spec
        .resources
        .max_processes
        .map(|value| value.to_string())
        .unwrap_or_else(|| "not required".into());
    let timeout = spec
        .resources
        .timeout_ms
        .map(|value| format!("{:.1}s", value as f64 / 1000.0))
        .unwrap_or_else(|| "not configured".into());

    [
        format!("Sandbox: {mode}"),
        format!("Backend: {backend}"),
        format!("Network: {network}"),
        format!("Filesystem: workspace {workspace_access} + explicit runtime mounts + temp"),
        format!("Memory: {memory}"),
        format!("Processes: {processes}"),
        format!("Timeout: {timeout}"),
        "Enforcement: verified per execution receipt; required unsupported capabilities fail closed"
            .into(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_status_does_not_claim_enforcement_from_configuration_alone() {
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(
            root.path(),
            Some(&SandboxSettings {
                mode: Some("workspace_write".into()),
                ..Default::default()
            }),
            None,
            false,
        )
        .unwrap()
        .unwrap();
        let status = format_sandbox_status(Some(&spec));
        assert!(status.contains("Sandbox: workspace_write"));
        assert!(status.contains("Network: denied"));
        assert!(status.contains("verified per execution receipt"));
        assert!(!status.contains("State: active"));

        let disabled = format_sandbox_status(None);
        assert!(disabled.contains("disabled (compatibility mode)"));
        assert!(disabled.contains("none claimed"));
    }

    #[test]
    fn container_runtime_and_image_are_user_authority_not_project_authority() {
        let global = SandboxSettings {
            mode: Some("workspace_write".into()),
            backend: Some("container".into()),
            container: Some(SandboxContainerSettings {
                runtime: Some("podman".into()),
                image: Some("example.invalid/davinci:rust-1.83".into()),
            }),
            ..Default::default()
        };
        let mut project = global.clone();
        project.container.as_mut().unwrap().image = Some("malicious/repo-image:latest".into());
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
            .unwrap_err()
            .contains("cannot change container runtime or image"));

        let spec = resolve_sandbox_settings(root.path(), Some(&global), None, false)
            .unwrap()
            .unwrap();
        assert_eq!(spec.backend, SandboxBackendKind::Container);
        assert_eq!(
            spec.container.as_ref().map(|value| value.runtime),
            Some(ContainerRuntime::Podman)
        );
    }

    #[test]
    fn project_policy_can_only_narrow_global_authority() {
        let global = SandboxSettings {
            mode: Some("workspace_write".into()),
            backend: Some("auto".into()),
            container: None,
            network: Some(SandboxNetworkSettings {
                mode: Some("deny".into()),
                ..Default::default()
            }),
            resources: Some(SandboxResourceSettings {
                timeout_seconds: Some(300),
                max_memory_mb: Some(4096),
                max_processes: Some(128),
            }),
        };
        let project = SandboxSettings {
            mode: Some("restricted".into()),
            backend: None,
            container: None,
            network: Some(SandboxNetworkSettings {
                mode: Some("deny".into()),
                ..Default::default()
            }),
            resources: Some(SandboxResourceSettings {
                timeout_seconds: Some(120),
                max_memory_mb: Some(2048),
                max_processes: Some(64),
            }),
        };
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
            .unwrap()
            .unwrap();
        assert_eq!(spec.mode, SandboxMode::Restricted);
        assert_eq!(spec.resources.timeout_ms, Some(120_000));
        assert_eq!(
            spec.resources.max_memory_bytes,
            Some(2048_u64 * 1024 * 1024)
        );
        assert_eq!(spec.resources.max_processes, Some(64));
    }

    #[test]
    fn project_cannot_enable_full_access_or_unrestricted_network() {
        let global = SandboxSettings {
            mode: Some("workspace_write".into()),
            ..Default::default()
        };
        let project = SandboxSettings {
            mode: Some("full_access".into()),
            network: Some(SandboxNetworkSettings {
                mode: Some("unrestricted".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        let error = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
            .unwrap_err();
        assert!(error.contains("cannot widen"), "{error}");
    }

    #[test]
    fn untrusted_project_sandbox_settings_are_ignored() {
        let global = SandboxSettings {
            mode: Some("restricted".into()),
            ..Default::default()
        };
        let project = SandboxSettings {
            mode: Some("full_access".into()),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), false)
            .unwrap()
            .unwrap();
        assert_eq!(spec.mode, SandboxMode::Restricted);
    }

    #[test]
    fn absent_global_sandbox_keeps_legacy_execution_explicitly_disabled() {
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_sandbox_settings(root.path(), None, None, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn workspace_write_defaults_to_denied_network_and_minimal_home() {
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(
            root.path(),
            Some(&SandboxSettings {
                mode: Some("workspace_write".into()),
                ..Default::default()
            }),
            None,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(spec.network, NetworkPolicy::Denied));
        assert_eq!(
            spec.environment.inject.get("HOME").map(String::as_str),
            Some("/tmp/davinci-home")
        );
        assert!(spec.required_capabilities.filesystem_isolation);
        assert!(spec.required_capabilities.network_denied);
        assert!(spec.required_capabilities.environment_isolation);
    }

    #[test]
    #[cfg(unix)]
    fn runtime_mount_discovery_never_exposes_the_user_home_root() {
        let workspace = tempfile::tempdir().unwrap();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let mut sources = Vec::new();
        push_runtime_source(&mut sources, &home, workspace.path());
        assert!(sources.is_empty(), "user home root must never become a runtime mount");
    }

    #[test]
    fn configured_hard_limits_become_required_backend_capabilities() {
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(
            root.path(),
            Some(&SandboxSettings {
                mode: Some("workspace_write".into()),
                resources: Some(SandboxResourceSettings {
                    timeout_seconds: Some(10),
                    max_memory_mb: Some(512),
                    max_processes: Some(32),
                }),
                ..Default::default()
            }),
            None,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(spec.required_capabilities.memory_limit);
        assert!(spec.required_capabilities.pid_limit);
    }
}
