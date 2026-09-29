//! Deterministic sandbox policy resolution for untrusted subprocesses.
//!
//! This module does not treat permission/approval as confinement. It converts a
//! host-resolved `SandboxSpec` into a backend launch plan and fails closed when
//! a requested property is unavailable. The actual child is still spawned by
//! the supervised executor helper so process-tree ownership is established
//! before untrusted code can run.

use davinci_protocol::{
    EnvironmentPolicy, ExecutionRequest, MountAccess, NetworkPolicy, SandboxBackendKind,
    SandboxCapabilities, SandboxErrorCode, SandboxFailure, SandboxId, SandboxMode, SandboxSpec,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const BASELINE_ENVIRONMENT: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SYSTEMROOT",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExecution {
    pub sandbox_id: SandboxId,
    pub spec_digest: String,
    pub backend: SandboxBackendKind,
    pub capabilities: SandboxCapabilities,
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
}

pub trait SandboxBackend {
    fn kind(&self) -> SandboxBackendKind;
    fn capabilities(&self) -> SandboxCapabilities;
    fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
    ) -> Result<PreparedExecution, SandboxFailure>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HostBackend;

impl SandboxBackend for HostBackend {
    fn kind(&self) -> SandboxBackendKind {
        SandboxBackendKind::Host
    }

    fn capabilities(&self) -> SandboxCapabilities {
        lifecycle_capabilities()
    }

    fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
    ) -> Result<PreparedExecution, SandboxFailure> {
        validate_request(spec, request)?;
        if spec.mode != SandboxMode::FullAccess {
            return Err(SandboxFailure::capability_unavailable(
                "host backend is available only for explicit full_access mode",
            ));
        }
        if !matches!(spec.network, NetworkPolicy::Unrestricted) {
            return Err(SandboxFailure::capability_unavailable(
                "host backend cannot enforce network restrictions",
            ));
        }
        let capabilities = self.capabilities();
        require_capabilities(spec, capabilities)?;
        Ok(PreparedExecution {
            sandbox_id: spec.id.clone(),
            spec_digest: sandbox_spec_digest(spec)?,
            backend: self.kind(),
            capabilities,
            executable: PathBuf::from(&request.executable),
            argv: request.argv.clone(),
            cwd: PathBuf::from(&request.cwd),
            environment: environment.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct LinuxBubblewrapBackend {
    executable: PathBuf,
}

impl LinuxBubblewrapBackend {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    fn workspace_mount_is_valid(&self, spec: &SandboxSpec) -> bool {
        spec.filesystem.mounts.iter().any(|mount| {
            mount.source.as_deref() == Some(spec.workspace.as_str())
                && mount.target == spec.workspace
                && match spec.mode {
                    SandboxMode::Restricted => mount.access == MountAccess::ReadOnly,
                    SandboxMode::WorkspaceWrite => mount.access == MountAccess::ReadWrite,
                    SandboxMode::NoExecution | SandboxMode::FullAccess => false,
                }
        })
    }
}

impl SandboxBackend for LinuxBubblewrapBackend {
    fn kind(&self) -> SandboxBackendKind {
        SandboxBackendKind::LinuxBubblewrap
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            process_tree_isolation: true,
            ephemeral_root: true,
            ephemeral_temp: true,
            output_limit: true,
            timeout: true,
            deterministic_teardown: true,
            ..Default::default()
        }
    }

    fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
    ) -> Result<PreparedExecution, SandboxFailure> {
        validate_request(spec, request)?;
        if !matches!(
            spec.mode,
            SandboxMode::Restricted | SandboxMode::WorkspaceWrite
        ) {
            return Err(SandboxFailure::capability_unavailable(
                "bubblewrap backend is for restricted/workspace_write execution",
            ));
        }
        if !self.workspace_mount_is_valid(spec) {
            return Err(SandboxFailure::new(
                SandboxErrorCode::FilesystemDenied,
                "sandbox mode requires an exact workspace mount",
            ));
        }
        if matches!(spec.network, NetworkPolicy::AllowList { .. }) {
            return Err(SandboxFailure::capability_unavailable(
                "bubblewrap backend does not implement DNS-safe domain allowlisting",
            ));
        }
        let capabilities = self.capabilities();
        require_capabilities(spec, capabilities)?;

        let mut argv = vec![
            "--die-with-parent".into(),
            "--new-session".into(),
            "--unshare-user-try".into(),
            "--unshare-pid".into(),
            "--unshare-ipc".into(),
            "--unshare-uts".into(),
            "--proc".into(),
            "/proc".into(),
            "--dev".into(),
            "/dev".into(),
            "--tmpfs".into(),
            "/tmp".into(),
            "--clearenv".into(),
        ];
        if matches!(spec.network, NetworkPolicy::Denied) {
            argv.push("--unshare-net".into());
        }

        for mount in &spec.filesystem.mounts {
            match mount.access {
                MountAccess::ReadOnly => {
                    argv.push("--ro-bind".into());
                    argv.push(mount.source.clone().expect("validated source"));
                    argv.push(mount.target.clone());
                }
                MountAccess::ReadWrite => {
                    argv.push("--bind".into());
                    argv.push(mount.source.clone().expect("validated source"));
                    argv.push(mount.target.clone());
                }
                MountAccess::Temporary => {
                    argv.push("--tmpfs".into());
                    argv.push(mount.target.clone());
                }
                // The bubblewrap root contains only declared mounts, so omitted
                // host paths are already hidden. Do not materialize them.
                MountAccess::Hidden => {}
            }
        }

        for (name, value) in environment {
            argv.push("--setenv".into());
            argv.push(name.clone());
            argv.push(value.clone());
        }
        argv.push("--chdir".into());
        argv.push(request.cwd.clone());
        argv.push("--".into());
        argv.push(request.executable.clone());
        argv.extend(request.argv.clone());

        Ok(PreparedExecution {
            sandbox_id: spec.id.clone(),
            spec_digest: sandbox_spec_digest(spec)?,
            backend: self.kind(),
            capabilities,
            executable: self.executable.clone(),
            argv,
            cwd: PathBuf::from("/"),
            // Child environment is set by bubblewrap arguments. The wrapper
            // receives no ambient credentials.
            environment: BTreeMap::new(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct SandboxBroker;

impl SandboxBroker {
    pub fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
    ) -> Result<PreparedExecution, SandboxFailure> {
        spec.validate()?;
        if spec.mode == SandboxMode::NoExecution {
            return Err(SandboxFailure::policy_denied(
                "sandbox mode does not permit process execution",
            ));
        }
        match spec.backend {
            SandboxBackendKind::Host => HostBackend.prepare(spec, request, environment),
            SandboxBackendKind::LinuxBubblewrap => {
                let path = find_host_executable("bwrap", Path::new(&spec.workspace)).ok_or_else(
                    || {
                        SandboxFailure::new(
                            SandboxErrorCode::SandboxUnavailable,
                            "bubblewrap executable is unavailable outside the workspace",
                        )
                    },
                )?;
                LinuxBubblewrapBackend::new(path).prepare(spec, request, environment)
            }
            SandboxBackendKind::Container => Err(SandboxFailure::new(
                SandboxErrorCode::SandboxUnavailable,
                "container backend requires explicit runtime/image wiring",
            )),
            SandboxBackendKind::Auto => {
                if cfg!(target_os = "linux") {
                    if let Some(path) =
                        find_host_executable("bwrap", Path::new(&spec.workspace))
                    {
                        return LinuxBubblewrapBackend::new(path)
                            .prepare(spec, request, environment);
                    }
                }
                if spec.mode == SandboxMode::FullAccess {
                    HostBackend.prepare(spec, request, environment)
                } else {
                    Err(SandboxFailure::new(
                        SandboxErrorCode::SandboxUnavailable,
                        "no backend can enforce the requested sandbox policy",
                    ))
                }
            }
        }
    }
}

pub fn sanitize_environment_from(
    policy: &EnvironmentPolicy,
    parent: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SandboxFailure> {
    let mut child = BTreeMap::new();
    for name in BASELINE_ENVIRONMENT.iter().copied().chain(policy.allow.iter().map(String::as_str)) {
        if let Some(value) = parent.get(name) {
            child.insert(name.to_string(), value.clone());
        }
    }
    for (name, value) in &policy.inject {
        validate_env_name(name)?;
        if value.contains('\0') {
            return Err(SandboxFailure::policy_denied(
                "sandbox environment value contains NUL",
            ));
        }
        child.insert(name.clone(), value.clone());
    }
    Ok(child)
}

pub fn sanitize_current_environment(
    policy: &EnvironmentPolicy,
) -> Result<BTreeMap<String, String>, SandboxFailure> {
    let parent = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect::<BTreeMap<_, _>>();
    sanitize_environment_from(policy, &parent)
}

/// Deterministically attenuate a delegated sandbox request. The child may
/// reduce authority but cannot widen mode, network, host mounts, environment,
/// backend choice, background execution, or resource ceilings.
pub fn attenuate_child_spec(
    parent: &SandboxSpec,
    requested: &SandboxSpec,
) -> Result<SandboxSpec, SandboxFailure> {
    parent.validate()?;
    requested.validate()?;
    if !requested.mode.is_no_more_permissive_than(parent.mode) {
        return Err(SandboxFailure::policy_denied(
            "child sandbox mode exceeds parent authority",
        ));
    }
    if requested.workspace != parent.workspace {
        return Err(SandboxFailure::policy_denied(
            "child workspace requires a separately delegated sandbox",
        ));
    }
    if !network_is_no_more_permissive(&parent.network, &requested.network) {
        return Err(SandboxFailure::policy_denied(
            "child network policy exceeds parent authority",
        ));
    }
    for mount in &requested.filesystem.mounts {
        if matches!(mount.access, MountAccess::Temporary | MountAccess::Hidden) {
            continue;
        }
        let allowed = parent.filesystem.mounts.iter().any(|candidate| {
            candidate.source == mount.source
                && candidate.target == mount.target
                && mount_access_is_no_more_permissive(mount.access, candidate.access)
        });
        if !allowed {
            return Err(SandboxFailure::policy_denied(
                "child filesystem mount exceeds parent authority",
            ));
        }
    }
    for name in &requested.environment.allow {
        if !parent.environment.allow.contains(name) {
            return Err(SandboxFailure::policy_denied(
                "child environment allowlist exceeds parent authority",
            ));
        }
    }
    for (name, value) in &requested.environment.inject {
        if parent.environment.inject.get(name) != Some(value) {
            return Err(SandboxFailure::policy_denied(
                "child environment injection exceeds parent authority",
            ));
        }
    }

    let backend = match (parent.backend, requested.backend) {
        (parent, SandboxBackendKind::Auto) => parent,
        (SandboxBackendKind::Auto, child) if child != SandboxBackendKind::Host => child,
        (parent, child) if parent == child => child,
        _ => {
            return Err(SandboxFailure::policy_denied(
                "child backend selection exceeds parent authority",
            ))
        }
    };

    if requested.process.allow_background && !parent.process.allow_background {
        return Err(SandboxFailure::policy_denied(
            "child cannot enable background processes",
        ));
    }

    let mut effective = requested.clone();
    effective.backend = backend;
    effective.resources.timeout_ms =
        bounded_u64(parent.resources.timeout_ms, requested.resources.timeout_ms, "timeout")?;
    effective.resources.cpu_time_ms = bounded_u64(
        parent.resources.cpu_time_ms,
        requested.resources.cpu_time_ms,
        "cpu time",
    )?;
    effective.resources.max_memory_bytes = bounded_u64(
        parent.resources.max_memory_bytes,
        requested.resources.max_memory_bytes,
        "memory",
    )?;
    effective.resources.max_output_bytes = bounded_u64(
        parent.resources.max_output_bytes,
        requested.resources.max_output_bytes,
        "output",
    )?;
    effective.resources.max_file_bytes = bounded_u64(
        parent.resources.max_file_bytes,
        requested.resources.max_file_bytes,
        "file size",
    )?;
    effective.resources.max_temp_bytes = bounded_u64(
        parent.resources.max_temp_bytes,
        requested.resources.max_temp_bytes,
        "temporary storage",
    )?;
    effective.resources.max_processes = bounded_u32(
        parent.resources.max_processes,
        requested.resources.max_processes,
        "process count",
    )?;
    effective.process.allow_background =
        parent.process.allow_background && requested.process.allow_background;
    effective.process.max_background_lifetime_ms = bounded_u64(
        parent.process.max_background_lifetime_ms,
        requested.process.max_background_lifetime_ms,
        "background lifetime",
    )?;
    effective.required_capabilities =
        capability_union(parent.required_capabilities, requested.required_capabilities);
    Ok(effective)
}

fn require_capabilities(
    spec: &SandboxSpec,
    available: SandboxCapabilities,
) -> Result<(), SandboxFailure> {
    if available.satisfies(&spec.required_capabilities) {
        Ok(())
    } else {
        Err(SandboxFailure::capability_unavailable(
            "selected backend cannot enforce all mandatory sandbox capabilities",
        ))
    }
}

fn validate_request(
    spec: &SandboxSpec,
    request: &ExecutionRequest,
) -> Result<(), SandboxFailure> {
    spec.validate()?;
    if request.sandbox_id != spec.id {
        return Err(SandboxFailure::new(
            SandboxErrorCode::ProtocolFailure,
            "execution request sandbox identity mismatch",
        ));
    }
    let executable = Path::new(&request.executable);
    let cwd = Path::new(&request.cwd);
    let workspace = Path::new(&spec.workspace);
    if !executable.is_absolute() {
        return Err(SandboxFailure::policy_denied(
            "executor requires a host-resolved absolute executable",
        ));
    }
    if !cwd.is_absolute() || !cwd.starts_with(workspace) {
        return Err(SandboxFailure::new(
            SandboxErrorCode::FilesystemDenied,
            "execution cwd is outside the sandbox workspace",
        ));
    }
    if request.argv.len() > 256
        || request.argv.iter().any(|arg| arg.contains('\0'))
        || request
            .argv
            .iter()
            .map(String::len)
            .sum::<usize>()
            > 64 * 1024
    {
        return Err(SandboxFailure::policy_denied(
            "invalid or oversized executor argv",
        ));
    }
    Ok(())
}

fn find_host_executable(name: &str, workspace: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
        let candidate = directory.join(name);
        if let Ok(canonical) = candidate.canonicalize() {
            if canonical.is_file() && !canonical.starts_with(workspace) {
                return Some(canonical);
            }
        }
    }
    None
}

pub fn sandbox_spec_digest(spec: &SandboxSpec) -> Result<String, SandboxFailure> {
    let mut redacted = spec.clone();
    for value in redacted.environment.inject.values_mut() {
        *value = "[injected]".into();
    }
    let bytes = serde_json::to_vec(&redacted).map_err(|_| {
        SandboxFailure::new(
            SandboxErrorCode::ProtocolFailure,
            "cannot encode sandbox policy digest",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn lifecycle_capabilities() -> SandboxCapabilities {
    SandboxCapabilities {
        environment_isolation: true,
        process_tree_isolation: true,
        output_limit: true,
        timeout: true,
        deterministic_teardown: true,
        ..Default::default()
    }
}

fn validate_env_name(name: &str) -> Result<(), SandboxFailure> {
    if name.is_empty()
        || name.len() > 128
        || name.as_bytes()[0].is_ascii_digit()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Err(SandboxFailure::policy_denied(
            "invalid sandbox environment variable name",
        ))
    } else {
        Ok(())
    }
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
            child_domains.iter().all(|value| parent_domains.contains(value))
                && child_ports.iter().all(|value| parent_ports.contains(value))
        }
        (NetworkPolicy::Unrestricted, _) => true,
        _ => false,
    }
}

fn mount_access_is_no_more_permissive(child: MountAccess, parent: MountAccess) -> bool {
    matches!(
        (child, parent),
        (MountAccess::ReadOnly, MountAccess::ReadOnly | MountAccess::ReadWrite)
            | (MountAccess::ReadWrite, MountAccess::ReadWrite)
            | (MountAccess::Temporary, MountAccess::Temporary)
            | (MountAccess::Hidden, _)
    )
}

fn bounded_u64(
    parent: Option<u64>,
    child: Option<u64>,
    name: &str,
) -> Result<Option<u64>, SandboxFailure> {
    match (parent, child) {
        (Some(parent), Some(child)) if child > parent => Err(SandboxFailure::policy_denied(
            format!("child {name} limit is weaker than parent"),
        )),
        (Some(parent), None) => Ok(Some(parent)),
        (_, child) => Ok(child),
    }
}

fn bounded_u32(
    parent: Option<u32>,
    child: Option<u32>,
    name: &str,
) -> Result<Option<u32>, SandboxFailure> {
    match (parent, child) {
        (Some(parent), Some(child)) if child > parent => Err(SandboxFailure::policy_denied(
            format!("child {name} limit is weaker than parent"),
        )),
        (Some(parent), None) => Ok(Some(parent)),
        (_, child) => Ok(child),
    }
}

fn capability_union(
    left: SandboxCapabilities,
    right: SandboxCapabilities,
) -> SandboxCapabilities {
    SandboxCapabilities {
        filesystem_isolation: left.filesystem_isolation || right.filesystem_isolation,
        network_denied: left.network_denied || right.network_denied,
        network_allowlist: left.network_allowlist || right.network_allowlist,
        environment_isolation: left.environment_isolation || right.environment_isolation,
        process_tree_isolation: left.process_tree_isolation || right.process_tree_isolation,
        pid_limit: left.pid_limit || right.pid_limit,
        memory_limit: left.memory_limit || right.memory_limit,
        cpu_limit: left.cpu_limit || right.cpu_limit,
        ephemeral_root: left.ephemeral_root || right.ephemeral_root,
        ephemeral_temp: left.ephemeral_temp || right.ephemeral_temp,
        output_limit: left.output_limit || right.output_limit,
        timeout: left.timeout || right.timeout,
        deterministic_teardown: left.deterministic_teardown || right.deterministic_teardown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::{
        EnvironmentPolicy, FilesystemPolicy, MountAccess, MountRule, NetworkPolicy,
        ProcessPolicy, ResourcePolicy, SandboxBackendKind, SandboxCapabilities, SandboxId,
        SandboxMode, SandboxSpec,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn ambient_secrets_are_not_in_the_default_environment() {
        let parent = BTreeMap::from([
            ("PATH".into(), "/bin".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("GITHUB_TOKEN".into(), "secret".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "secret".into()),
            ("SSH_AUTH_SOCK".into(), "/tmp/agent.sock".into()),
            ("PRIVATE_VALUE".into(), "private".into()),
        ]);
        let policy = EnvironmentPolicy::default();
        let child = sanitize_environment_from(&policy, &parent).unwrap();
        assert_eq!(child.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(child.get("LANG").map(String::as_str), Some("C.UTF-8"));
        assert!(!child.contains_key("GITHUB_TOKEN"));
        assert!(!child.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!child.contains_key("SSH_AUTH_SOCK"));
        assert!(!child.contains_key("PRIVATE_VALUE"));
    }

    #[test]
    fn child_policy_cannot_enable_network_denied_by_parent() {
        let parent = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let child = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Unrestricted);
        let error = attenuate_child_spec(&parent, &child).unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::PolicyDenied);
    }

    #[test]
    fn child_resource_limits_only_tighten() {
        let mut parent = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        parent.resources.max_memory_bytes = Some(1024);
        let mut child = parent.clone();
        child.resources.max_memory_bytes = Some(2048);
        assert_eq!(
            attenuate_child_spec(&parent, &child).unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
        child.resources.max_memory_bytes = Some(512);
        assert_eq!(
            attenuate_child_spec(&parent, &child)
                .unwrap()
                .resources
                .max_memory_bytes,
            Some(512)
        );
    }

    #[test]
    fn host_backend_is_only_for_full_access() {
        let backend = HostBackend;
        let spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let request = request(&spec);
        let error = backend
            .prepare(&spec, &request, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::CapabilityUnavailable);
    }

    #[test]
    fn bubblewrap_denied_network_uses_network_namespace() {
        let backend = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"));
        let spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let request = request(&spec);
        let prepared = backend
            .prepare(
                &spec,
                &request,
                &BTreeMap::from([("PATH".into(), "/usr/bin".into())]),
            )
            .unwrap();
        assert_eq!(prepared.executable, PathBuf::from("/usr/bin/bwrap"));
        assert!(prepared.argv.iter().any(|arg| arg == "--unshare-net"));
        assert!(prepared.argv.windows(3).any(|args| {
            args == [String::from("--bind"), spec.workspace.clone(), spec.workspace.clone()]
        }));
    }

    #[test]
    fn bubblewrap_does_not_claim_domain_allowlisting() {
        let backend = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"));
        let mut spec = spec(
            SandboxMode::WorkspaceWrite,
            NetworkPolicy::AllowList {
                domains: vec!["example.com".into()],
                ports: vec![443],
            },
        );
        spec.required_capabilities.network_allowlist = true;
        let error = backend
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::CapabilityUnavailable);
    }

    #[test]
    fn no_execution_mode_fails_before_backend_spawn() {
        let broker = SandboxBroker;
        let spec = spec(SandboxMode::NoExecution, NetworkPolicy::Denied);
        let error = broker
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::PolicyDenied);
    }

    fn request(spec: &SandboxSpec) -> ExecutionRequest {
        ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: if cfg!(windows) {
                "C:\\Windows\\System32\\cmd.exe".into()
            } else {
                "/bin/sh".into()
            },
            argv: vec!["-c".into(), "true".into()],
            cwd: spec.workspace.clone(),
        }
    }

    fn spec(mode: SandboxMode, network: NetworkPolicy) -> SandboxSpec {
        let workspace = if cfg!(windows) {
            "C:\\workspace".to_string()
        } else {
            "/workspace".to_string()
        };
        SandboxSpec {
            id: SandboxId("sbx-test".into()),
            mode,
            backend: if mode == SandboxMode::FullAccess {
                SandboxBackendKind::Host
            } else {
                SandboxBackendKind::LinuxBubblewrap
            },
            workspace: workspace.clone(),
            filesystem: FilesystemPolicy {
                mounts: vec![MountRule {
                    source: Some(workspace.clone()),
                    target: workspace,
                    access: if mode == SandboxMode::Restricted {
                        MountAccess::ReadOnly
                    } else {
                        MountAccess::ReadWrite
                    },
                }],
            },
            network,
            environment: EnvironmentPolicy::default(),
            resources: ResourcePolicy::default(),
            process: ProcessPolicy::default(),
            required_capabilities: SandboxCapabilities {
                filesystem_isolation: mode != SandboxMode::FullAccess,
                network_denied: mode != SandboxMode::FullAccess,
                environment_isolation: true,
                process_tree_isolation: true,
                deterministic_teardown: true,
                ..Default::default()
            },
        }
    }
    #[test]
    fn worker_workspace_rebind_preserves_parent_authority() {
        let parent_root = tempfile::tempdir().unwrap();
        let child_root = tempfile::tempdir().unwrap();
        let parent_path = parent_root.path().canonicalize().unwrap();
        let child_path = child_root.path().canonicalize().unwrap();

        let mut parent = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        parent.workspace = parent_path.to_string_lossy().into_owned();
        parent.filesystem.mounts = vec![MountRule {
            source: Some(parent.workspace.clone()),
            target: parent.workspace.clone(),
            access: MountAccess::ReadWrite,
        }];
        parent.resources.max_memory_bytes = Some(1024);
        parent.process.allow_background = true;

        let child = rebind_worker_spec(
            &parent,
            &child_path,
            SandboxId("worker-child".into()),
            true,
        )
        .unwrap();

        assert_eq!(child.id.0, "worker-child");
        assert_eq!(child.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(child.network, NetworkPolicy::Denied);
        assert_eq!(child.resources.max_memory_bytes, Some(1024));
        assert_eq!(child.process.allow_background, parent.process.allow_background);
        assert_eq!(child.workspace, child_path.to_string_lossy());
        assert!(child.filesystem.mounts.iter().any(|mount| {
            mount.source.as_deref() == Some(child.workspace.as_str())
                && mount.target == child.workspace
                && mount.access == MountAccess::ReadWrite
        }));
        assert!(!child.filesystem.mounts.iter().any(|mount| {
            mount.source.as_deref() == Some(parent.workspace.as_str())
                && mount.access == MountAccess::ReadWrite
        }));
    }

    #[test]
    fn read_only_worker_cannot_gain_workspace_write() {
        let parent_root = tempfile::tempdir().unwrap();
        let child_root = tempfile::tempdir().unwrap();
        let parent_path = parent_root.path().canonicalize().unwrap();
        let child_path = child_root.path().canonicalize().unwrap();

        let mut parent = spec(SandboxMode::Restricted, NetworkPolicy::Denied);
        parent.workspace = parent_path.to_string_lossy().into_owned();
        parent.filesystem.mounts = vec![MountRule {
            source: Some(parent.workspace.clone()),
            target: parent.workspace.clone(),
            access: MountAccess::ReadOnly,
        }];

        let child = rebind_worker_spec(
            &parent,
            &child_path,
            SandboxId("worker-ro".into()),
            true,
        )
        .unwrap();
        assert_eq!(child.mode, SandboxMode::Restricted);
        assert!(child.filesystem.mounts.iter().any(|mount| {
            mount.source.as_deref() == Some(child.workspace.as_str())
                && mount.access == MountAccess::ReadOnly
        }));
    }

    #[test]
    fn isolated_worker_rejects_unrelated_parent_write_mounts() {
        let parent_root = tempfile::tempdir().unwrap();
        let child_root = tempfile::tempdir().unwrap();
        let shared_root = tempfile::tempdir().unwrap();
        let parent_path = parent_root.path().canonicalize().unwrap();
        let child_path = child_root.path().canonicalize().unwrap();
        let shared_path = shared_root.path().canonicalize().unwrap();

        let mut parent = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        parent.workspace = parent_path.to_string_lossy().into_owned();
        parent.filesystem.mounts = vec![
            MountRule {
                source: Some(parent.workspace.clone()),
                target: parent.workspace.clone(),
                access: MountAccess::ReadWrite,
            },
            MountRule {
                source: Some(shared_path.to_string_lossy().into_owned()),
                target: shared_path.to_string_lossy().into_owned(),
                access: MountAccess::ReadWrite,
            },
        ];

        assert_eq!(
            rebind_worker_spec(
                &parent,
                &child_path,
                SandboxId("worker-denied".into()),
                true,
            )
            .unwrap_err()
            .code,
            SandboxErrorCode::PolicyDenied
        );
    }

}
