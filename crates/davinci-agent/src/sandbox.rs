//! Deterministic sandbox policy resolution for untrusted subprocesses.
//!
//! This module does not treat permission/approval as confinement. It converts a
//! host-resolved `SandboxSpec` into a backend launch plan and fails closed when
//! a requested property is unavailable. The actual child is still spawned by
//! the supervised executor helper. Capability negotiation must establish
//! process-tree ownership before a policy requiring it can run.

use davinci_protocol::{
    ContainerRuntime, EnvironmentPolicy, ExecutionRequest, MountAccess, NetworkPolicy,
    SandboxBackendKind, SandboxCapabilities, SandboxErrorCode, SandboxFailure, SandboxId,
    SandboxMode, SandboxSpec,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod seatbelt;
pub use seatbelt::MacosSeatbeltBackend;

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
            memory_limit: true,
            cpu_limit: true,
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
        // After the writable workspace bind, so these read-only binds sit on top.
        for path in protected_workspace_paths(spec) {
            argv.push("--ro-bind".into());
            argv.push(path.clone());
            argv.push(path);
        }

        // The executor helper already starts this wrapper with env_clear()
        // and exactly the sanitized environment. Let bubblewrap inherit it;
        // putting values in --setenv argv would expose them through host
        // process inspection.
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
            // The executor helper clears ambient credentials before launching
            // bubblewrap. The sandbox inherits only this sanitized map.
            environment: environment.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ContainerBackend {
    executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerCleanupPlan {
    pub executable: PathBuf,
    pub name: String,
}

impl ContainerBackend {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    fn container_name(spec: &SandboxSpec, launch_id: Option<&str>) -> String {
        let raw = match launch_id {
            Some(launch) => format!("{}-{launch}", spec.id.0),
            None => spec.id.0.clone(),
        };
        let suffix = raw
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                    ch
                } else {
                    '-'
                }
            })
            .collect::<String>();
        format!("davinci-{}", suffix)
    }
}

impl SandboxBackend for ContainerBackend {
    fn kind(&self) -> SandboxBackendKind {
        SandboxBackendKind::Container
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            process_tree_isolation: true,
            pid_limit: true,
            memory_limit: true,
            cpu_limit: true,
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
                "container backend is for restricted/workspace_write execution",
            ));
        }
        let container = spec.container.as_ref().ok_or_else(|| {
            SandboxFailure::new(
                SandboxErrorCode::SandboxUnavailable,
                "container backend requires trusted runtime/image configuration",
            )
        })?;
        if matches!(spec.network, NetworkPolicy::AllowList { .. }) {
            return Err(SandboxFailure::capability_unavailable(
                "container backend does not implement DNS-safe domain allowlisting",
            ));
        }
        let capabilities = self.capabilities();
        require_capabilities(spec, capabilities)?;

        let mut argv = vec![
            "run".into(),
            "--name".into(),
            Self::container_name(spec, request.launch_id.as_deref()),
            "--rm".into(),
            "--init".into(),
            "--read-only".into(),
        ];
        if matches!(spec.network, NetworkPolicy::Denied) {
            argv.extend(["--network".into(), "none".into()]);
        }
        argv.extend(["--workdir".into(), request.cwd.clone()]);

        for mount in &spec.filesystem.mounts {
            match mount.access {
                MountAccess::ReadOnly | MountAccess::ReadWrite => {
                    let source = mount.source.as_ref().expect("validated mount source");
                    if source.contains(',') || mount.target.contains(',') {
                        return Err(SandboxFailure::new(
                            SandboxErrorCode::FilesystemDenied,
                            "container mount paths containing commas are unsupported",
                        ));
                    }
                    let mut value = format!("type=bind,src={source},dst={}", mount.target);
                    if mount.access == MountAccess::ReadOnly {
                        value.push_str(",readonly");
                    }
                    argv.extend(["--mount".into(), value]);
                }
                MountAccess::Temporary => {
                    let mut value = format!("{}:rw,nosuid,nodev", mount.target);
                    if let Some(bytes) = spec.resources.max_temp_bytes {
                        value.push_str(&format!(",size={bytes}"));
                    }
                    argv.extend(["--tmpfs".into(), value]);
                }
                MountAccess::Hidden => {}
            }
        }
        for path in protected_workspace_paths(spec) {
            if path.contains(',') {
                return Err(SandboxFailure::new(
                    SandboxErrorCode::FilesystemDenied,
                    "container mount paths containing commas are unsupported",
                ));
            }
            argv.extend([
                "--mount".into(),
                format!("type=bind,src={path},dst={path},readonly"),
            ]);
        }

        for name in environment.keys() {
            // Docker/Podman copy the value from their own sanitized environment.
            // Values never appear in argv/process listings.
            argv.extend(["--env".into(), name.clone()]);
        }
        if let Some(bytes) = spec.resources.max_memory_bytes {
            argv.extend(["--memory".into(), bytes.to_string()]);
        }
        if let Some(processes) = spec.resources.max_processes {
            argv.extend(["--pids-limit".into(), processes.to_string()]);
        }
        if let Some(milliseconds) = spec.resources.cpu_time_ms {
            let seconds = milliseconds.saturating_add(999) / 1000;
            argv.extend(["--ulimit".into(), format!("cpu={0}:{0}", seconds.max(1))]);
        }
        if let Some(bytes) = spec.resources.max_file_bytes {
            argv.extend(["--ulimit".into(), format!("fsize={bytes}:{bytes}")]);
        }

        argv.push(container.image.clone());
        argv.push(request.executable.clone());
        argv.extend(request.argv.clone());

        Ok(PreparedExecution {
            sandbox_id: spec.id.clone(),
            spec_digest: sandbox_spec_digest(spec)?,
            backend: self.kind(),
            capabilities,
            executable: self.executable.clone(),
            argv,
            cwd: PathBuf::from(&spec.workspace),
            environment: environment.clone(),
        })
    }
}

pub fn container_cleanup_plan(
    spec: &SandboxSpec,
    launch_id: Option<&str>,
) -> Option<ContainerCleanupPlan> {
    if spec.backend != SandboxBackendKind::Container
        && !(spec.backend == SandboxBackendKind::Auto && spec.container.is_some())
    {
        return None;
    }
    let container = spec.container.as_ref()?;
    let runtime = resolve_container_runtime(container.runtime, Path::new(&spec.workspace))?;
    Some(ContainerCleanupPlan {
        executable: runtime,
        name: ContainerBackend::container_name(spec, launch_id),
    })
}

fn resolve_container_runtime(runtime: ContainerRuntime, workspace: &Path) -> Option<PathBuf> {
    match runtime {
        ContainerRuntime::Docker => find_host_executable("docker", workspace),
        ContainerRuntime::Podman => find_host_executable("podman", workspace),
        ContainerRuntime::Auto => find_host_executable("podman", workspace)
            .or_else(|| find_host_executable("docker", workspace)),
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
        self.prepare_with_private_temp(spec, request, environment, None)
    }

    pub(crate) fn prepare_with_private_temp(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
        private_temp: Option<&Path>,
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
                let path =
                    find_host_executable("bwrap", Path::new(&spec.workspace)).ok_or_else(|| {
                        SandboxFailure::new(
                            SandboxErrorCode::SandboxUnavailable,
                            "bubblewrap executable is unavailable outside the workspace",
                        )
                    })?;
                LinuxBubblewrapBackend::new(path).prepare(spec, request, environment)
            }
            SandboxBackendKind::MacosSeatbelt => {
                if !cfg!(target_os = "macos") {
                    return Err(SandboxFailure::new(
                        SandboxErrorCode::SandboxUnavailable,
                        "Seatbelt requires native macOS",
                    ));
                }
                MacosSeatbeltBackend::new(private_temp).prepare(spec, request, environment)
            }
            SandboxBackendKind::Container => {
                let container = spec.container.as_ref().ok_or_else(|| {
                    SandboxFailure::new(
                        SandboxErrorCode::SandboxUnavailable,
                        "container backend requires trusted runtime/image configuration",
                    )
                })?;
                let runtime =
                    resolve_container_runtime(container.runtime, Path::new(&spec.workspace))
                        .ok_or_else(|| {
                            SandboxFailure::new(
                        SandboxErrorCode::SandboxUnavailable,
                        "configured Docker/Podman runtime is unavailable outside the workspace",
                    )
                        })?;
                ContainerBackend::new(runtime).prepare(spec, request, environment)
            }
            SandboxBackendKind::Auto => {
                if spec.mode == SandboxMode::FullAccess {
                    return HostBackend.prepare(spec, request, environment);
                }
                if cfg!(target_os = "macos")
                    && MacosSeatbeltBackend::new(private_temp)
                        .capabilities()
                        .satisfies(&spec.required_capabilities)
                {
                    return MacosSeatbeltBackend::new(private_temp).prepare(
                        spec,
                        request,
                        environment,
                    );
                }
                if cfg!(target_os = "linux") {
                    if let Some(path) = find_host_executable("bwrap", Path::new(&spec.workspace)) {
                        return LinuxBubblewrapBackend::new(path).prepare(
                            spec,
                            request,
                            environment,
                        );
                    }
                }
                if let Some(container) = spec.container.as_ref() {
                    if let Some(runtime) =
                        resolve_container_runtime(container.runtime, Path::new(&spec.workspace))
                    {
                        return ContainerBackend::new(runtime).prepare(spec, request, environment);
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
    for name in BASELINE_ENVIRONMENT
        .iter()
        .copied()
        .chain(policy.allow.iter().map(String::as_str))
    {
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

    if requested.container != parent.container {
        return Err(SandboxFailure::policy_denied(
            "child cannot change container runtime or image authority",
        ));
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
    effective.resources.timeout_ms = bounded_u64(
        parent.resources.timeout_ms,
        requested.resources.timeout_ms,
        "timeout",
    )?;
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
    effective.required_capabilities = capability_union(
        parent.required_capabilities,
        requested.required_capabilities,
    );
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

fn validate_request(spec: &SandboxSpec, request: &ExecutionRequest) -> Result<(), SandboxFailure> {
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
        || request.argv.iter().map(String::len).sum::<usize>() > 64 * 1024
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

pub fn rebind_worker_spec(
    parent: &SandboxSpec,
    child_workspace: &Path,
    child_id: SandboxId,
    allow_workspace_write: bool,
) -> Result<SandboxSpec, SandboxFailure> {
    parent.validate()?;
    let child_workspace = child_workspace.canonicalize().map_err(|error| {
        SandboxFailure::new(
            SandboxErrorCode::FilesystemDenied,
            format!("worker workspace is unavailable: {error}"),
        )
    })?;
    let child_workspace = child_workspace
        .to_str()
        .ok_or_else(|| SandboxFailure::policy_denied("worker workspace path is not UTF-8"))?
        .to_string();

    let mode = match parent.mode {
        SandboxMode::NoExecution => SandboxMode::NoExecution,
        SandboxMode::Restricted => SandboxMode::Restricted,
        SandboxMode::WorkspaceWrite if allow_workspace_write => SandboxMode::WorkspaceWrite,
        SandboxMode::WorkspaceWrite => SandboxMode::Restricted,
        SandboxMode::FullAccess => SandboxMode::FullAccess,
    };

    let mut mounts = Vec::with_capacity(parent.filesystem.mounts.len());
    let mut rebound_workspace = false;
    for mount in &parent.filesystem.mounts {
        let is_parent_workspace = mount.source.as_deref() == Some(parent.workspace.as_str())
            && mount.target == parent.workspace
            && matches!(mount.access, MountAccess::ReadOnly | MountAccess::ReadWrite);
        if is_parent_workspace {
            rebound_workspace = true;
            mounts.push(davinci_protocol::MountRule {
                source: Some(child_workspace.clone()),
                target: child_workspace.clone(),
                access: if mode == SandboxMode::WorkspaceWrite {
                    MountAccess::ReadWrite
                } else {
                    MountAccess::ReadOnly
                },
            });
            continue;
        }
        if mount.access == MountAccess::ReadWrite {
            return Err(SandboxFailure::policy_denied(
                "isolated worker cannot inherit an unrelated writable host mount",
            ));
        }
        mounts.push(mount.clone());
    }

    if matches!(
        parent.mode,
        SandboxMode::Restricted | SandboxMode::WorkspaceWrite
    ) && !rebound_workspace
    {
        return Err(SandboxFailure::new(
            SandboxErrorCode::FilesystemDenied,
            "parent sandbox has no exact workspace mount to delegate",
        ));
    }

    let mut child = parent.clone();
    child.id = child_id;
    child.mode = mode;
    child.workspace = child_workspace.clone();
    child.filesystem.mounts = mounts;
    child.validate()?;

    // Defense in depth: the delegated spec must be no wider than the parent
    // with its workspace moved to the worker's, on every dimension.
    let mut ceiling = parent.clone();
    ceiling.id = child.id.clone();
    ceiling.workspace = child_workspace.clone();
    for mount in &mut ceiling.filesystem.mounts {
        if mount.source.as_deref() == Some(parent.workspace.as_str())
            && mount.target == parent.workspace
        {
            mount.source = Some(child_workspace.clone());
            mount.target = child_workspace.clone();
        }
    }
    attenuate_child_spec(&ceiling, &child)
}

/// Workspace paths a sandboxed process must not modify even when the
/// workspace itself is writable, because the host later acts on them outside
/// the sandbox: `.git` (config such as `core.fsmonitor`, hooks and filters run
/// when Davinci or the user runs `git`) and project configuration (`.davinci`,
/// `.pi`: hooks, MCP servers, extensions, settings). Evaluated per launch, so
/// a repository created by an earlier command is protected from then on.
pub fn protected_workspace_paths(spec: &SandboxSpec) -> Vec<String> {
    if spec.mode != SandboxMode::WorkspaceWrite {
        return Vec::new();
    }
    let workspace = Path::new(&spec.workspace);
    [".git", ".davinci", ".pi"]
        .into_iter()
        .filter_map(|name| {
            let path = workspace.join(name);
            // A symlink would bind its target, not the link; the link itself
            // stays replaceable, so refuse to guess and protect only real
            // entries (a symlinked `.git` pointing outside the workspace is
            // not mounted at all).
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink() {
                return None;
            }
            path.to_str().map(str::to_string)
        })
        .collect()
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
            child_domains
                .iter()
                .all(|value| parent_domains.contains(value))
                && child_ports.iter().all(|value| parent_ports.contains(value))
        }
        (NetworkPolicy::Unrestricted, _) => true,
        _ => false,
    }
}

fn mount_access_is_no_more_permissive(child: MountAccess, parent: MountAccess) -> bool {
    matches!(
        (child, parent),
        (
            MountAccess::ReadOnly,
            MountAccess::ReadOnly | MountAccess::ReadWrite
        ) | (MountAccess::ReadWrite, MountAccess::ReadWrite)
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

fn capability_union(left: SandboxCapabilities, right: SandboxCapabilities) -> SandboxCapabilities {
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
        EnvironmentPolicy, FilesystemPolicy, MountAccess, MountRule, NetworkPolicy, ProcessPolicy,
        ResourcePolicy, SandboxBackendKind, SandboxCapabilities, SandboxId, SandboxMode,
        SandboxSpec,
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
    fn auto_full_access_uses_explicit_host_even_with_native_backend_installed() {
        let mut spec = spec(SandboxMode::FullAccess, NetworkPolicy::Unrestricted);
        spec.required_capabilities = lifecycle_capabilities();
        let prepared = SandboxBroker
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap();
        assert_eq!(prepared.backend, SandboxBackendKind::Host);
        assert!(!prepared.capabilities.filesystem_isolation);
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
            args == [
                String::from("--bind"),
                spec.workspace.clone(),
                spec.workspace.clone(),
            ]
        }));
    }

    #[test]
    fn workspace_write_keeps_git_and_project_config_read_only() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().canonicalize().unwrap();
        std::fs::create_dir(workspace.join(".git")).unwrap();
        std::fs::create_dir(workspace.join(".davinci")).unwrap();
        let mut spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        spec.workspace = workspace.to_string_lossy().into_owned();
        spec.filesystem.mounts[0].source = Some(spec.workspace.clone());
        spec.filesystem.mounts[0].target = spec.workspace.clone();
        let argv = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"))
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap()
            .argv;
        let position = |args: [String; 3]| argv.windows(3).position(|window| window == args);
        let git = workspace.join(".git").to_string_lossy().into_owned();
        let config = workspace.join(".davinci").to_string_lossy().into_owned();
        let bind = position([
            "--bind".into(),
            spec.workspace.clone(),
            spec.workspace.clone(),
        ])
        .expect("workspace bind");
        let git_bind = position(["--ro-bind".into(), git.clone(), git]).expect("read-only .git");
        assert!(
            git_bind > bind,
            "the read-only bind must sit on top of the workspace"
        );
        assert!(position(["--ro-bind".into(), config.clone(), config]).is_some());

        // Read-only mode has nothing writable to protect.
        spec.mode = SandboxMode::Restricted;
        assert!(protected_workspace_paths(&spec).is_empty());
    }

    #[test]
    fn container_names_are_unique_per_launch() {
        let mut spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        spec.backend = SandboxBackendKind::Container;
        spec.container = Some(davinci_protocol::ContainerPolicy {
            runtime: ContainerRuntime::Docker,
            image: "example/image".into(),
        });
        let backend = ContainerBackend::new(PathBuf::from("/usr/bin/docker"));
        let name = |launch: &str| {
            let mut request = request(&spec);
            request.launch_id = Some(launch.into());
            let argv = backend
                .prepare(&spec, &request, &BTreeMap::new())
                .unwrap()
                .argv;
            let at = argv.iter().position(|arg| arg == "--name").unwrap();
            argv[at + 1].clone()
        };
        assert_ne!(name("a"), name("b"));
        assert_eq!(
            name("a"),
            ContainerBackend::container_name(&spec, Some("a"))
        );
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
            launch_id: None,
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
            container: None,
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

        let child =
            rebind_worker_spec(&parent, &child_path, SandboxId("worker-child".into()), true)
                .unwrap();

        assert_eq!(child.id.0, "worker-child");
        assert_eq!(child.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(child.network, NetworkPolicy::Denied);
        assert_eq!(child.resources.max_memory_bytes, Some(1024));
        assert_eq!(
            child.process.allow_background,
            parent.process.allow_background
        );
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

        let child =
            rebind_worker_spec(&parent, &child_path, SandboxId("worker-ro".into()), true).unwrap();
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

/// Availability is an execution result, not a PATH lookup. Cache the bounded
/// probe for this host process; explicit policies still fail closed at launch.
pub fn detected_native_backend(workspace: &Path) -> Option<SandboxBackendKind> {
    static AVAILABLE: std::sync::OnceLock<Option<SandboxBackendKind>> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| probe_native_backend(workspace))
}

/// Auto requires both confinement and ownership. A path-policy backend alone
/// must not enable autonomy while descendants can escape the supervisor.
pub fn supports_auto_sandbox(backend: SandboxBackendKind) -> bool {
    let required = SandboxCapabilities {
        filesystem_isolation: true,
        network_denied: true,
        ephemeral_temp: true,
        ..lifecycle_capabilities()
    };
    let capabilities = match backend {
        SandboxBackendKind::LinuxBubblewrap => {
            LinuxBubblewrapBackend::new(PathBuf::from("bwrap")).capabilities()
        }
        SandboxBackendKind::MacosSeatbelt => MacosSeatbeltBackend::new(None).capabilities(),
        _ => return false,
    };
    capabilities.satisfies(&required)
}

fn probe_native_backend(workspace: &Path) -> Option<SandboxBackendKind> {
    if cfg!(target_os = "linux") {
        let executable = find_host_executable("bwrap", workspace)?;
        let mut command = std::process::Command::new(executable);
        command.args([
            "--die-with-parent",
            "--new-session",
            "--unshare-user-try",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--",
            "/bin/true",
        ]);
        return bounded_probe(command, std::time::Duration::from_secs(2))
            .then_some(SandboxBackendKind::LinuxBubblewrap);
    }
    if cfg!(target_os = "macos") {
        // XNU allows setsid/setpgid without Seatbelt mediation. Until the
        // execution plane owns detached descendants, native Auto is disabled.
        if !supports_auto_sandbox(SandboxBackendKind::MacosSeatbelt) {
            return None;
        }
        // The profile parser, runtime paths, and deny-default execution must
        // all work; sandbox-exec merely existing is not sufficient.
        let directory = PrivateTemp::new(Path::new("/private/tmp")).ok()?;
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).ok()?;
        let workspace = workspace.canonicalize().ok()?;
        let temp = directory.path().join("temp");
        std::fs::create_dir(&temp).ok()?;
        std::fs::create_dir(temp.join("home")).ok()?;
        let text = workspace.to_str()?.to_string();
        let mut spec = SandboxSpec {
            id: SandboxId("native-probe".into()),
            mode: SandboxMode::WorkspaceWrite,
            backend: SandboxBackendKind::MacosSeatbelt,
            container: None,
            workspace: text.clone(),
            filesystem: Default::default(),
            network: NetworkPolicy::Denied,
            environment: Default::default(),
            resources: Default::default(),
            process: Default::default(),
            required_capabilities: Default::default(),
        };
        for path in [
            "/System",
            "/System/Volumes/Preboot/Cryptexes/OS/System",
            "/usr",
            "/bin",
            "/sbin",
        ] {
            if let Ok(source) = Path::new(path).canonicalize() {
                let source = source.to_str()?.to_string();
                spec.filesystem.mounts.push(davinci_protocol::MountRule {
                    source: Some(source.clone()),
                    target: source,
                    access: MountAccess::ReadOnly,
                });
            }
        }
        spec.filesystem.mounts.push(davinci_protocol::MountRule {
            source: Some(text.clone()),
            target: text.clone(),
            access: MountAccess::ReadWrite,
        });
        let request = ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: "/bin/sh".into(),
            argv: vec![
                "-c".into(),
                "printf usable > probe && /bin/cat probe >/dev/null".into(),
            ],
            cwd: text,
            launch_id: None,
        };
        let prepared = MacosSeatbeltBackend::new(Some(&temp))
            .prepare(&spec, &request, &BTreeMap::new())
            .ok()?;
        let mut command = std::process::Command::new(prepared.executable);
        command
            .args(prepared.argv)
            .current_dir(prepared.cwd)
            .envs(prepared.environment);
        return bounded_probe(command, std::time::Duration::from_secs(2))
            .then_some(SandboxBackendKind::MacosSeatbelt);
    }
    None
}

fn bounded_probe(mut command: std::process::Command, timeout: std::time::Duration) -> bool {
    use std::{process::Stdio, time::Instant};
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Launch-owned native temp. UUID naming + exclusive mkdir + mode 0700 keep
/// the directory private without adding a production dependency.
pub(crate) struct PrivateTemp {
    path: PathBuf,
}
impl PrivateTemp {
    pub(crate) fn new(parent: &Path) -> std::io::Result<Self> {
        let path = parent.join(format!(
            "davinci-seatbelt-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&path)?;
        let mut directory = Self { path };
        directory.path = directory.path.canonicalize()?;
        builder.create(directory.path.join("home"))?;
        Ok(directory)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn close(self) -> std::io::Result<()> {
        std::fs::remove_dir_all(&self.path).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })
    }
}
impl Drop for PrivateTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod native_probe_tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn probe_checks_exit_success_and_is_bounded() {
        let mut failure = std::process::Command::new("/bin/sh");
        failure.args(["-c", "exit 1"]);
        assert!(!bounded_probe(failure, std::time::Duration::from_secs(1)));
        let mut slow = std::process::Command::new("/bin/sh");
        slow.args(["-c", "exec /bin/sleep 10"]);
        let start = std::time::Instant::now();
        assert!(!bounded_probe(slow, std::time::Duration::from_millis(100)));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert!(bounded_probe(
            std::process::Command::new("/usr/bin/true"),
            std::time::Duration::from_secs(1)
        ));
    }

    #[test]
    fn private_temp_is_distinct_exclusive_and_cleaned() {
        let parent = tempfile::tempdir().unwrap();
        let first = PrivateTemp::new(parent.path()).unwrap();
        let second = PrivateTemp::new(parent.path()).unwrap();
        assert_ne!(first.path(), second.path());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(first.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let path = first.path().to_path_buf();
        std::fs::write(path.join("home/state"), "data").unwrap();
        first.close().unwrap();
        assert!(!path.exists());
        assert!(second.path().exists());
    }
}
