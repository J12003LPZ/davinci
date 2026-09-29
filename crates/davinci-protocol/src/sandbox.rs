use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    NoExecution,
    Restricted,
    WorkspaceWrite,
    FullAccess,
}

impl SandboxMode {
    pub fn privilege_rank(self) -> u8 {
        match self {
            Self::NoExecution => 0,
            Self::Restricted => 1,
            Self::WorkspaceWrite => 2,
            Self::FullAccess => 3,
        }
    }

    pub fn is_no_more_permissive_than(self, parent: Self) -> bool {
        self.privilege_rank() <= parent.privilege_rank()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackendKind {
    Auto,
    LinuxBubblewrap,
    Container,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerRuntime {
    Auto,
    Docker,
    Podman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerPolicy {
    pub runtime: ContainerRuntime,
    pub image: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxLifecycle {
    Created,
    Ready,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SandboxId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountAccess {
    ReadOnly,
    ReadWrite,
    Temporary,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub target: String,
    pub access: MountAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FilesystemPolicy {
    pub mounts: Vec<MountRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetworkPolicy {
    Denied,
    AllowList {
        #[serde(default)]
        domains: Vec<String>,
        #[serde(default)]
        ports: Vec<u16>,
    },
    Unrestricted,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self::Denied
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentPolicy {
    /// Variable names that the trusted broker may copy from its own environment.
    pub allow: Vec<String>,
    /// Explicit values supplied by the trusted broker. Values are never receipt fields.
    pub inject: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ResourcePolicy {
    pub timeout_ms: Option<u64>,
    pub cpu_time_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub max_processes: Option<u32>,
    pub max_output_bytes: Option<u64>,
    pub max_file_bytes: Option<u64>,
    pub max_temp_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessPolicy {
    pub allow_background: bool,
    pub max_background_lifetime_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxCapabilities {
    pub filesystem_isolation: bool,
    pub network_denied: bool,
    pub network_allowlist: bool,
    pub environment_isolation: bool,
    pub process_tree_isolation: bool,
    pub pid_limit: bool,
    pub memory_limit: bool,
    pub cpu_limit: bool,
    pub ephemeral_root: bool,
    pub ephemeral_temp: bool,
    pub output_limit: bool,
    pub timeout: bool,
    pub deterministic_teardown: bool,
}

impl SandboxCapabilities {
    pub fn satisfies(&self, required: &Self) -> bool {
        (!required.filesystem_isolation || self.filesystem_isolation)
            && (!required.network_denied || self.network_denied)
            && (!required.network_allowlist || self.network_allowlist)
            && (!required.environment_isolation || self.environment_isolation)
            && (!required.process_tree_isolation || self.process_tree_isolation)
            && (!required.pid_limit || self.pid_limit)
            && (!required.memory_limit || self.memory_limit)
            && (!required.cpu_limit || self.cpu_limit)
            && (!required.ephemeral_root || self.ephemeral_root)
            && (!required.ephemeral_temp || self.ephemeral_temp)
            && (!required.output_limit || self.output_limit)
            && (!required.timeout || self.timeout)
            && (!required.deterministic_teardown || self.deterministic_teardown)
    }

    pub fn is_subset_of(&self, parent: &Self) -> bool {
        parent.satisfies(self)
    }

    pub fn intersection(&self, other: &Self) -> Self {
        Self {
            filesystem_isolation: self.filesystem_isolation && other.filesystem_isolation,
            network_denied: self.network_denied && other.network_denied,
            network_allowlist: self.network_allowlist && other.network_allowlist,
            environment_isolation: self.environment_isolation && other.environment_isolation,
            process_tree_isolation: self.process_tree_isolation && other.process_tree_isolation,
            pid_limit: self.pid_limit && other.pid_limit,
            memory_limit: self.memory_limit && other.memory_limit,
            cpu_limit: self.cpu_limit && other.cpu_limit,
            ephemeral_root: self.ephemeral_root && other.ephemeral_root,
            ephemeral_temp: self.ephemeral_temp && other.ephemeral_temp,
            output_limit: self.output_limit && other.output_limit,
            timeout: self.timeout && other.timeout,
            deterministic_teardown: self.deterministic_teardown && other.deterministic_teardown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSpec {
    pub id: SandboxId,
    pub mode: SandboxMode,
    pub backend: SandboxBackendKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerPolicy>,
    pub workspace: String,
    #[serde(default)]
    pub filesystem: FilesystemPolicy,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub environment: EnvironmentPolicy,
    #[serde(default)]
    pub resources: ResourcePolicy,
    #[serde(default)]
    pub process: ProcessPolicy,
    #[serde(default)]
    pub required_capabilities: SandboxCapabilities,
}

impl SandboxSpec {
    pub fn validate(&self) -> Result<(), SandboxFailure> {
        if self.id.0.is_empty() || self.id.0.len() > 128 || self.id.0.chars().any(char::is_control)
        {
            return Err(SandboxFailure::policy_denied("invalid sandbox id"));
        }
        validate_absolute_path(&self.workspace, "workspace")?;
        if let Some(container) = &self.container {
            let image = container.image.trim();
            if image.is_empty()
                || image.len() > 512
                || image.chars().any(char::is_control)
                || image.chars().any(char::is_whitespace)
            {
                return Err(SandboxFailure::policy_denied(
                    "invalid container image reference",
                ));
            }
        }
        for mount in &self.filesystem.mounts {
            validate_absolute_path(&mount.target, "mount target")?;
            match mount.access {
                MountAccess::ReadOnly | MountAccess::ReadWrite => {
                    let source = mount
                        .source
                        .as_deref()
                        .ok_or_else(|| SandboxFailure::policy_denied("mount source is required"))?;
                    validate_absolute_path(source, "mount source")?;
                }
                MountAccess::Temporary | MountAccess::Hidden => {
                    if mount.source.is_some() {
                        return Err(SandboxFailure::policy_denied(
                            "temporary/hidden mounts cannot name a host source",
                        ));
                    }
                }
            }
        }
        for name in self
            .environment
            .allow
            .iter()
            .chain(self.environment.inject.keys())
        {
            validate_environment_name(name)?;
        }
        if self
            .environment
            .inject
            .values()
            .any(|value| value.contains('\0'))
        {
            return Err(SandboxFailure::policy_denied(
                "environment values cannot contain NUL",
            ));
        }
        if let NetworkPolicy::AllowList { domains, ports } = &self.network {
            if domains.is_empty()
                || domains.iter().any(|domain| {
                    let domain = domain.trim();
                    domain.is_empty()
                        || domain.len() > 253
                        || domain.chars().any(|c| {
                            c.is_control()
                                || c.is_whitespace()
                                || matches!(c, '/' | '\\' | '@' | '#')
                        })
                })
                || ports.iter().any(|port| *port == 0)
            {
                return Err(SandboxFailure::policy_denied("invalid network allowlist"));
            }
        }
        for (value, name) in [
            (self.resources.timeout_ms, "timeout"),
            (self.resources.cpu_time_ms, "cpu time"),
            (self.resources.max_memory_bytes, "memory limit"),
            (self.resources.max_output_bytes, "output limit"),
            (self.resources.max_file_bytes, "file limit"),
            (self.resources.max_temp_bytes, "temp limit"),
            (
                self.process.max_background_lifetime_ms,
                "background lifetime",
            ),
        ] {
            if value == Some(0) {
                return Err(SandboxFailure::policy_denied(format!(
                    "{name} must be greater than zero"
                )));
            }
        }
        if self.resources.max_processes == Some(0) {
            return Err(SandboxFailure::policy_denied(
                "process limit must be greater than zero",
            ));
        }
        Ok(())
    }
}

fn validate_absolute_path(value: &str, label: &str) -> Result<(), SandboxFailure> {
    if value.is_empty() || value.contains('\0') || !is_absolute_portable(value) {
        return Err(SandboxFailure::policy_denied(format!(
            "{label} must be an absolute path"
        )));
    }
    if has_parent_component(value) {
        return Err(SandboxFailure::policy_denied(format!(
            "{label} contains parent traversal"
        )));
    }
    Ok(())
}

fn is_absolute_portable(value: &str) -> bool {
    if value.starts_with('/') || value.starts_with("\\") {
        return true;
    }
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn has_parent_component(value: &str) -> bool {
    value.split(['/', '\\']).any(|component| component == "..")
}

fn validate_environment_name(name: &str) -> Result<(), SandboxFailure> {
    if name.is_empty()
        || name.len() > 128
        || name.as_bytes()[0].is_ascii_digit()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(SandboxFailure::policy_denied(
            "invalid environment variable name",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    pub sandbox_id: SandboxId,
    pub executable: String,
    #[serde(default)]
    pub argv: Vec<String>,
    pub cwd: String,
    /// Unique per launch. Backends that name host-visible resources (a
    /// container) derive the name from it so concurrent launches under one
    /// sandbox policy never collide or tear each other down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    pub sandbox_id: SandboxId,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub resource_limited: bool,
    pub output_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxReceipt {
    pub sandbox_id: SandboxId,
    pub spec_digest: String,
    pub backend: SandboxBackendKind,
    pub capabilities: SandboxCapabilities,
    pub lifecycle: SandboxLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxErrorCode {
    SandboxUnavailable,
    CapabilityUnavailable,
    PolicyDenied,
    FilesystemDenied,
    NetworkDenied,
    ResourceLimitExceeded,
    ExecutionTimedOut,
    SandboxCrashed,
    ProtocolFailure,
    CleanupFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxFailure {
    pub code: SandboxErrorCode,
    pub message: String,
}

impl SandboxFailure {
    pub fn new(code: SandboxErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn policy_denied(message: impl Into<String>) -> Self {
        Self::new(SandboxErrorCode::PolicyDenied, message)
    }

    pub fn capability_unavailable(message: impl Into<String>) -> Self {
        Self::new(SandboxErrorCode::CapabilityUnavailable, message)
    }
}

impl std::fmt::Display for SandboxFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxFailure {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn required_capabilities_fail_closed() {
        let available = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: false,
            environment_isolation: true,
            process_tree_isolation: true,
            timeout: true,
            output_limit: true,
            ..Default::default()
        };
        let required = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            ..Default::default()
        };
        assert!(!available.satisfies(&required));
    }

    #[test]
    fn child_capabilities_are_intersection_only() {
        let parent = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            process_tree_isolation: true,
            memory_limit: true,
            ..Default::default()
        };
        let requested = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: false,
            environment_isolation: true,
            process_tree_isolation: true,
            cpu_limit: true,
            ..Default::default()
        };
        let child = parent.intersection(&requested);
        assert!(child.is_subset_of(&parent));
        assert!(child.filesystem_isolation);
        assert!(!child.network_denied);
        assert!(!child.cpu_limit);
    }

    #[test]
    fn mount_traversal_is_rejected() {
        let mut spec = fixture_spec();
        spec.filesystem.mounts.push(MountRule {
            source: Some("/host/workspace".into()),
            target: "/project/../escape".into(),
            access: MountAccess::ReadWrite,
        });
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::PolicyDenied);
    }

    #[test]
    fn allowlist_requires_real_destinations() {
        let mut spec = fixture_spec();
        spec.network = NetworkPolicy::AllowList {
            domains: vec!["".into(), "example.com".into()],
            ports: vec![443],
        };
        assert_eq!(
            spec.validate().unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn container_image_reference_is_bounded_and_token_safe() {
        let mut spec = fixture_spec();
        spec.backend = SandboxBackendKind::Container;
        spec.container = Some(ContainerPolicy {
            runtime: ContainerRuntime::Docker,
            image: "example.invalid/davinci-rust:1.83".into(),
        });
        assert!(spec.validate().is_ok());
        spec.container.as_mut().unwrap().image = "bad image".into();
        assert_eq!(
            spec.validate().unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn environment_names_are_validated() {
        let mut spec = fixture_spec();
        spec.environment.inject = BTreeMap::from([("BAD=NAME".into(), "secret".into())]);
        assert_eq!(
            spec.validate().unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn security_types_reject_unknown_fields() {
        let value = serde_json::json!({
            "id": "sbx-1",
            "mode": "restricted",
            "backend": "auto",
            "container": null,
            "workspace": "/workspace",
            "filesystem": {"mounts": []},
            "network": {"mode": "denied"},
            "environment": {"allow": [], "inject": {}},
            "resources": {},
            "process": {},
            "required_capabilities": {},
            "model_says_full_access": true
        });
        assert!(serde_json::from_value::<SandboxSpec>(value).is_err());
    }

    fn fixture_spec() -> SandboxSpec {
        SandboxSpec {
            id: SandboxId("sbx-1".into()),
            mode: SandboxMode::Restricted,
            backend: SandboxBackendKind::Auto,
            container: None,
            workspace: if cfg!(windows) {
                "C:\\workspace".into()
            } else {
                "/workspace".into()
            },
            filesystem: FilesystemPolicy::default(),
            network: NetworkPolicy::Denied,
            environment: EnvironmentPolicy::default(),
            resources: ResourcePolicy::default(),
            process: ProcessPolicy::default(),
            required_capabilities: SandboxCapabilities {
                filesystem_isolation: true,
                network_denied: true,
                environment_isolation: true,
                process_tree_isolation: true,
                ..Default::default()
            },
        }
    }
}
