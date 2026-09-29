use davinci_protocol::{
    ContainerPolicy, ContainerRuntime, EnvironmentPolicy, FilesystemPolicy, MountAccess, MountRule,
    NetworkPolicy, ProcessPolicy, ResourcePolicy, SandboxBackendKind, SandboxCapabilities,
    SandboxId, SandboxMode, SandboxSpec,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxSettings {
    pub mode: Option<String>,
    pub backend: Option<String>,
    pub container: Option<SandboxContainerSettings>,
    /// Host variable names passed through to sandboxed processes (and
    /// available to `${NAME}` references in sandboxed MCP `env`).
    pub environment: Option<SandboxEnvironmentSettings>,
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
pub struct SandboxEnvironmentSettings {
    pub allow: Vec<String>,
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
    if mode == SandboxMode::FullAccess
        && backend != SandboxBackendKind::Host
        && backend != SandboxBackendKind::Auto
    {
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
    let runtime = if matches!(mode, SandboxMode::Restricted | SandboxMode::WorkspaceWrite) {
        runtime_layout(&workspace)
    } else {
        RuntimeLayout::default()
    };
    let mut mounts = runtime.mounts;
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
            target: SANDBOX_HOME.into(),
            access: MountAccess::Temporary,
        });
        // Read-only toolchain caches inside the ephemeral home; they must
        // follow the tmpfs they are nested in.
        mounts.extend(runtime.home_mounts);
    }

    let mut environment = EnvironmentPolicy {
        allow: parse_environment_allow(effective.environment.as_ref())?,
        ..Default::default()
    };
    if !matches!(mode, SandboxMode::NoExecution | SandboxMode::FullAccess) {
        environment
            .inject
            .insert("HOME".into(), SANDBOX_HOME.into());
        environment.inject.insert("TMPDIR".into(), "/tmp".into());
        environment.inject.insert("TMP".into(), "/tmp".into());
        environment.inject.insert("TEMP".into(), "/tmp".into());
        environment.inject.extend(runtime.environment);
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

fn narrow_settings(global: &mut SandboxSettings, project: &SandboxSettings) -> Result<(), String> {
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
            return Err(
                "project sandbox cannot introduce container runtime/image authority".into(),
            );
        };
        if project_container != global_container {
            return Err("project sandbox cannot change container runtime or image".into());
        }
    }

    if let Some(project_environment) = project.environment.as_ref() {
        let global_allow = parse_environment_allow(global.environment.as_ref())?;
        let project_allow = parse_environment_allow(Some(project_environment))?;
        if project_allow
            .iter()
            .any(|name| !global_allow.contains(name))
        {
            return Err("project sandbox environment cannot widen global authority".into());
        }
        global.environment = Some(project_environment.clone());
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

fn parse_resources(settings: Option<&SandboxResourceSettings>) -> Result<ResourcePolicy, String> {
    let Some(settings) = settings else {
        // No sandbox-level deadline unless configured: the tool call's own
        // timeout governs, as it does without a sandbox.
        return Ok(ResourcePolicy {
            timeout_ms: None,
            max_output_bytes: Some(4 * 1024 * 1024),
            ..Default::default()
        });
    };
    let timeout_ms = settings
        .timeout_seconds
        .map(|seconds| seconds.checked_mul(1000).ok_or("sandbox timeout overflows"))
        .transpose()?;
    let max_memory_bytes = settings
        .max_memory_mb
        .map(|mb| {
            mb.checked_mul(1024 * 1024)
                .ok_or("sandbox memory limit overflows")
        })
        .transpose()?;
    if timeout_ms == Some(0) || max_memory_bytes == Some(0) || settings.max_processes == Some(0) {
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

const SANDBOX_HOME: &str = "/tmp/davinci-home";

fn parse_environment_allow(
    settings: Option<&SandboxEnvironmentSettings>,
) -> Result<Vec<String>, String> {
    let Some(settings) = settings else {
        return Ok(Vec::new());
    };
    let mut allow = Vec::new();
    for name in &settings.allow {
        let valid = !name.is_empty()
            && name.len() <= 128
            && !name.as_bytes()[0].is_ascii_digit()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !valid {
            return Err(format!("invalid sandbox environment variable name: {name}"));
        }
        if !allow.contains(name) {
            allow.push(name.clone());
        }
    }
    Ok(allow)
}

/// Host runtime a sandboxed command needs: read-only system and toolchain
/// mounts plus the environment that points tools at them.
#[derive(Default)]
struct RuntimeLayout {
    mounts: Vec<MountRule>,
    /// Mounted under the ephemeral `SANDBOX_HOME` tmpfs, so they must follow it.
    home_mounts: Vec<MountRule>,
    environment: std::collections::BTreeMap<String, String>,
}

/// Host paths a sandboxed process needs to execute ordinary tools.
///
/// Each path is mounted at its canonical location. A path that is itself a
/// symlink (`/lib64 -> usr/lib64` on usr-merged distributions, a
/// `/etc/resolv.conf` managed by systemd) is also mounted at its own
/// spelling, because nothing else in the sandbox root provides that link:
/// every dynamic executable names `/lib64/ld-linux-*.so` as its interpreter.
#[cfg(unix)]
const RUNTIME_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    // Debian/Ubuntu route awk, cc, c++, java and others through here.
    "/etc/alternatives",
    "/etc/ssl",
    "/etc/pki",
    "/etc/ca-certificates",
    "/etc/passwd",
    "/etc/group",
    "/etc/nsswitch.conf",
    // Name resolution when the network policy allows egress.
    "/etc/hosts",
    "/etc/host.conf",
    "/etc/resolv.conf",
    "/etc/localtime",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
];

fn runtime_layout(workspace: &Path) -> RuntimeLayout {
    let mut layout = RuntimeLayout::default();
    #[cfg(unix)]
    {
        let mut mounts = RuntimeMounts::default();
        for path in RUNTIME_PATHS {
            mounts.push(Path::new(path), workspace);
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                mounts.push(&directory, workspace);
            }
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        // rustup proxies in PATH look for toolchains under RUSTUP_HOME, which
        // defaults to the (replaced) HOME: point them at the mounted copy.
        if let Some(rustup) = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".rustup")))
        {
            if let Some(path) = mounts.push(&rustup, workspace) {
                layout.environment.insert("RUSTUP_HOME".into(), path);
            }
        }
        // Cargo needs a writable CARGO_HOME for its locks, so it lives in the
        // ephemeral home with the host's downloaded crates mounted read-only.
        if let Some(cargo) = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".cargo")))
        {
            mounts.push(&cargo.join("bin"), workspace);
            let sandbox_cargo = format!("{SANDBOX_HOME}/.cargo");
            let mut any = false;
            for relative in ["registry", "git", "config.toml", "config"] {
                let Ok(source) = cargo.join(relative).canonicalize() else {
                    continue;
                };
                if source.starts_with(workspace) || exposes_user_home_root(&source) {
                    continue;
                }
                let Some(source) = source.to_str() else {
                    continue;
                };
                any = true;
                layout.home_mounts.push(MountRule {
                    source: Some(source.to_string()),
                    target: format!("{sandbox_cargo}/{relative}"),
                    access: MountAccess::ReadOnly,
                });
            }
            if any {
                layout
                    .environment
                    .insert("CARGO_HOME".into(), sandbox_cargo);
            }
        }
        layout.mounts = mounts.into_rules();
    }
    #[cfg(not(unix))]
    let _ = workspace;
    layout
}

#[derive(Default)]
struct RuntimeMounts {
    /// Canonical host directories and files, mounted at the same path.
    canonical: Vec<PathBuf>,
    /// `(spelling, canonical)` for paths that are symlinks on the host.
    aliases: Vec<(PathBuf, PathBuf)>,
}

impl RuntimeMounts {
    /// Records `path`; returns its canonical spelling when it is mounted.
    fn push(&mut self, path: &Path, workspace: &Path) -> Option<String> {
        if !path.is_absolute() {
            return None;
        }
        let canonical = path.canonicalize().ok()?;
        if canonical.starts_with(workspace) || exposes_user_home_root(&canonical) {
            return None;
        }
        // Avoid redundant nested mounts when an already declared parent covers it.
        if !self
            .canonical
            .iter()
            .any(|seen| canonical.starts_with(seen))
        {
            self.canonical.retain(|seen| !seen.starts_with(&canonical));
            self.canonical.push(canonical.clone());
        }
        let spelling = lexical(path)?;
        if spelling != canonical && !self.aliases.iter().any(|(seen, _)| seen == &spelling) {
            self.aliases.push((spelling, canonical.clone()));
        }
        canonical.to_str().map(str::to_string)
    }

    fn into_rules(self) -> Vec<MountRule> {
        let mut rules = Vec::new();
        for path in &self.canonical {
            if let Some(path) = path.to_str() {
                rules.push(MountRule {
                    source: Some(path.to_string()),
                    target: path.to_string(),
                    access: MountAccess::ReadOnly,
                });
            }
        }
        for (spelling, canonical) in &self.aliases {
            // A link inside an already mounted tree (e.g. /usr/local/bin ->
            // /opt/...) resolves through that mount; only links whose
            // spelling nothing else provides need their own mount.
            if self.canonical.iter().any(|seen| spelling.starts_with(seen))
                || self
                    .aliases
                    .iter()
                    .any(|(other, _)| other != spelling && spelling.starts_with(other))
            {
                continue;
            }
            if let (Some(source), Some(target)) = (canonical.to_str(), spelling.to_str()) {
                rules.push(MountRule {
                    source: Some(source.to_string()),
                    target: target.to_string(),
                    access: MountAccess::ReadOnly,
                });
            }
        }
        rules
    }
}

/// `path` without `.` components; `None` when it has `..`.
fn lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
            other => out.push(other),
        }
    }
    Some(out)
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
            child_domains
                .iter()
                .all(|domain| parent_domains.contains(domain))
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
            return Err(format!(
                "project sandbox {name} limit cannot widen global authority"
            ));
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
            return Err(format!(
                "project sandbox {name} limit cannot widen global authority"
            ));
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
        assert!(
            resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
                .unwrap_err()
                .contains("cannot change container runtime or image")
        );

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
            environment: None,
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
            environment: None,
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
        let error =
            resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true).unwrap_err();
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
        let mut mounts = RuntimeMounts::default();
        assert!(mounts.push(&home, workspace.path()).is_none());
        assert!(
            mounts.into_rules().is_empty(),
            "user home root must never become a runtime mount"
        );
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_runtime_paths_are_also_mounted_at_their_own_spelling() {
        // Usr-merged hosts: /lib64 -> usr/lib64 holds the ELF interpreter of
        // every dynamic executable, and nothing else creates that link.
        let workspace = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        let host = host.path().canonicalize().unwrap();
        std::fs::create_dir_all(host.join("usr/lib64")).unwrap();
        std::os::unix::fs::symlink("usr/lib64", host.join("lib64")).unwrap();
        let mut mounts = RuntimeMounts::default();
        mounts.push(&host.join("usr"), workspace.path());
        mounts.push(&host.join("lib64"), workspace.path());
        let rules = mounts.into_rules();
        let text = |path: PathBuf| path.to_string_lossy().into_owned();
        assert!(rules.iter().any(|rule| {
            rule.source.as_deref() == Some(text(host.join("usr/lib64")).as_str())
                && rule.target == text(host.join("lib64"))
                && rule.access == MountAccess::ReadOnly
        }));
        // The canonical target is already covered by the /usr mount.
        assert_eq!(rules.len(), 2, "{rules:?}");
    }

    /// Runs the real launch plan under bubblewrap when this host can.
    #[test]
    #[cfg(target_os = "linux")]
    fn bubblewrap_workspace_write_runs_host_tools_and_keeps_git_read_only() {
        use davinci_agent::sandbox::SandboxBroker;
        let usable = std::process::Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--unshare-pid", "--", "true"])
            .output()
            .is_ok_and(|output| output.status.success());
        if !usable {
            eprintln!("skipping: bubblewrap is unavailable or cannot create namespaces here");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let git = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&workspace)
            .status();
        if !git.is_ok_and(|status| status.success()) {
            eprintln!("skipping: git is unavailable");
            return;
        }
        let settings = SandboxSettings {
            mode: Some("workspace_write".into()),
            backend: Some("bwrap".into()),
            ..Default::default()
        };
        let spec = resolve_sandbox_settings(&workspace, Some(&settings), None, false)
            .unwrap()
            .unwrap();
        let environment =
            davinci_agent::sandbox::sanitize_current_environment(&spec.environment).unwrap();
        let shell = ["/usr/bin/bash", "/bin/bash", "/usr/bin/sh", "/bin/sh"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .unwrap();
        let run = |script: &str| {
            let request = davinci_protocol::ExecutionRequest {
                sandbox_id: spec.id.clone(),
                executable: shell.into(),
                argv: vec!["-c".into(), script.into()],
                cwd: spec.workspace.clone(),
                launch_id: None,
            };
            let prepared = SandboxBroker
                .prepare(&spec, &request, &environment)
                .unwrap();
            std::process::Command::new(&prepared.executable)
                .args(&prepared.argv)
                .current_dir(&prepared.cwd)
                .env_clear()
                .envs(&prepared.environment)
                .output()
                .unwrap()
        };
        // The dynamic loader resolves: ordinary executables start.
        let output = run("echo started && echo data > file && cat file");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "started\ndata\n");
        // .git is not writable from inside, so its config cannot gain
        // commands the host later runs.
        let output = run("git config core.fsmonitor 'touch escaped'");
        assert!(!output.status.success());
        let config = std::fs::read_to_string(workspace.join(".git/config")).unwrap();
        assert!(!config.contains("fsmonitor"), "{config}");
    }

    #[test]
    fn project_environment_allowlist_can_only_narrow() {
        let root = tempfile::tempdir().unwrap();
        let global = SandboxSettings {
            environment: Some(SandboxEnvironmentSettings {
                allow: vec!["GITHUB_TOKEN".into(), "NPM_TOKEN".into()],
            }),
            ..Default::default()
        };
        let widen = SandboxSettings {
            environment: Some(SandboxEnvironmentSettings {
                allow: vec!["AWS_SECRET_ACCESS_KEY".into()],
            }),
            ..Default::default()
        };
        assert!(
            resolve_sandbox_settings(root.path(), Some(&global), Some(&widen), true)
                .unwrap_err()
                .contains("environment")
        );
        let narrow = SandboxSettings {
            environment: Some(SandboxEnvironmentSettings {
                allow: vec!["NPM_TOKEN".into()],
            }),
            ..Default::default()
        };
        let spec = resolve_sandbox_settings(root.path(), Some(&global), Some(&narrow), true)
            .unwrap()
            .unwrap();
        assert_eq!(spec.environment.allow, vec!["NPM_TOKEN".to_string()]);
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
