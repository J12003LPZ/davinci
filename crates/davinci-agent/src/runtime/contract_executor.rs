//! Confined execution capability interface for task contracts.
//!
//! Enforces provable process, filesystem, and external-effect boundaries.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::runtime::capabilities::{DeclaredEffect, PreparedAction};
use crate::runtime::contracts::{resolve_contract_path, ScopeViolation, TaskContract};

/// Core policy predicate governing whether an effect profile is permitted.
///
/// Invariant: An effect is permitted only when all effects are proven,
/// the backend explicitly enforces isolation, and all declared effects are authorized.
pub fn effect_profile_allows(
    effects_proven: bool,
    backend_enforces: bool,
    all_effects_authorized: bool,
) -> bool {
    effects_proven && backend_enforces && all_effects_authorized
}

/// Isolation capabilities owned by an execution backend.
///
/// The fields are intentionally private: production callers cannot manufacture a containment
/// claim by toggling booleans. Until a real backend constructor exists, `Default` represents the
/// ordinary unconfined host and therefore proves no isolation guarantees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutorCapabilities {
    process_isolation: bool,
    filesystem_sandbox: bool,
    network_sandbox: bool,
    can_enforce_process: bool,
    can_enforce_fs: bool,
    can_enforce_net: bool,
}

impl ExecutorCapabilities {
    #[cfg(test)]
    fn full_sandbox() -> Self {
        Self {
            process_isolation: true,
            filesystem_sandbox: true,
            network_sandbox: true,
            can_enforce_process: true,
            can_enforce_fs: true,
            can_enforce_net: true,
        }
    }
}

/// Errors returned when an execution contract cannot be proven or enforced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionError {
    ContractUnenforceable(String),
    ScopeViolation(ScopeViolation),
    UnauthorizedEffect(String),
    NetworkDisallowed(String),
    ShellSubstitutionBlocked(String),
    ArtifactRootViolation {
        path: String,
        allowed_roots: Vec<String>,
    },
    IrreversibleEffect(String),
    Other(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContractUnenforceable(msg) => {
                write!(f, "execution_contract_unenforceable: {msg}")
            }
            Self::ScopeViolation(v) => write!(f, "{v}"),
            Self::UnauthorizedEffect(msg) => write!(f, "unauthorized_effect: {msg}"),
            Self::NetworkDisallowed(msg) => write!(f, "network_disallowed: {msg}"),
            Self::ShellSubstitutionBlocked(msg) => {
                write!(f, "shell_substitution_blocked: {msg}")
            }
            Self::ArtifactRootViolation {
                path,
                allowed_roots,
            } => write!(
                f,
                "artifact_root_violation: path `{path}` is outside allowed roots: {allowed_roots:?}"
            ),
            Self::IrreversibleEffect(msg) => write!(f, "irreversible_effect: {msg}"),
            Self::Other(msg) => write!(f, "execution_error: {msg}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Confined execution manager validating actions and commands against task contracts.
#[derive(Debug, Clone)]
pub struct ContractExecutor {
    capabilities: ExecutorCapabilities,
    contract: Option<TaskContract>,
    trusted_workspace: PathBuf,
    allow_unconfined_shell: bool,
}

impl ContractExecutor {
    pub fn new(
        _backend_name: impl Into<String>,
        capabilities: ExecutorCapabilities,
        workspace: impl Into<PathBuf>,
    ) -> Self {
        Self {
            capabilities,
            contract: None,
            trusted_workspace: workspace.into(),
            allow_unconfined_shell: false,
        }
    }

    pub fn with_contract(mut self, contract: TaskContract) -> Self {
        self.contract = Some(contract);
        self
    }

    /// Let the ordinary host defer shell execution to the normal permission
    /// policy while still enforcing every statically checkable contract rule.
    pub fn allowing_unconfined_shell(mut self) -> Self {
        self.allow_unconfined_shell = true;
        self
    }

    pub fn capabilities(&self) -> &ExecutorCapabilities {
        &self.capabilities
    }

    /// Validates a prepared action under the active contract and backend capabilities.
    ///
    /// This policy layer never executes a process by itself. A real backend must own the spawn
    /// and containment operation; until one exists production capabilities remain `Default` and
    /// process/network effects are refused before dispatch.
    pub fn execute(&self, action: &PreparedAction) -> Result<(), ExecutionError> {
        let contract = self.contract.as_ref().ok_or_else(|| {
            ExecutionError::ContractUnenforceable(
                "contracted executor requires an explicit task contract".into(),
            )
        })?;

        if action.declared_effects.iter().any(|effect| {
            matches!(
                effect,
                DeclaredEffect::McpRead | DeclaredEffect::McpMutation | DeclaredEffect::Other(_)
            )
        }) {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot prove or contain remote/unknown declared effects".into(),
            ));
        }

        // Invariant 1: Backend must be able to enforce all declared effects
        let needs_process = action
            .declared_effects
            .contains(&DeclaredEffect::ProcessExecution);
        let needs_fs = action
            .declared_effects
            .contains(&DeclaredEffect::FileSystemWrite);
        let needs_net = action
            .declared_effects
            .contains(&DeclaredEffect::NetworkAccess);

        if needs_process && !self.capabilities.can_enforce_process {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot enforce process isolation constraints".into(),
            ));
        }
        if needs_fs && !self.capabilities.can_enforce_fs {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot enforce filesystem sandbox constraints".into(),
            ));
        }
        if needs_net && !self.capabilities.can_enforce_net {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot enforce network sandbox constraints".into(),
            ));
        }

        // Invariant 2: Network is default-deny unless explicitly granted in external_effects
        if needs_net {
            let net_authorized = contract
                .external_effects
                .iter()
                .any(|e| e == "network" || e == "network_access");

            if !net_authorized {
                return Err(ExecutionError::NetworkDisallowed(
                    "network access is not permitted under this contract (default deny)".into(),
                ));
            }
        }

        // Invariant 3: External publish / deploy is irreversible and requires explicit grant
        if action
            .declared_effects
            .contains(&DeclaredEffect::ExternalServicePublish)
        {
            let publish_authorized = contract
                .external_effects
                .iter()
                .any(|e| e == "publish" || e == "external_publish");

            if !publish_authorized {
                return Err(ExecutionError::UnauthorizedEffect(
                    "external service publish is irreversible and requires an explicit contract effect grant"
                        .into(),
                ));
            }
        }

        // Invariant 4: Filesystem targets must be in contract writable scope and not protected
        if needs_fs {
            if let Some(contract) = &self.contract {
                for target in &action.targets {
                    let rel =
                        resolve_contract_path(&self.trusted_workspace, target).map_err(|err| {
                            ExecutionError::ScopeViolation(ScopeViolation {
                                requested_target: target.clone(),
                                tool: action.tool.clone(),
                                writable_scope: contract.writable_paths.clone(),
                                protected_scope: contract.protected_paths.clone(),
                                reason: format!("path resolution failed: {err}"),
                            })
                        })?;

                    match contract.allows_path(&rel) {
                        Ok(true) => {}
                        Ok(false) => {
                            return Err(ExecutionError::ScopeViolation(ScopeViolation {
                                requested_target: rel,
                                tool: action.tool.clone(),
                                writable_scope: contract.writable_paths.clone(),
                                protected_scope: contract.protected_paths.clone(),
                                reason: "target path is not within writable scope or is protected"
                                    .into(),
                            }));
                        }
                        Err(err) => {
                            return Err(ExecutionError::ScopeViolation(ScopeViolation {
                                requested_target: rel,
                                tool: action.tool.clone(),
                                writable_scope: contract.writable_paths.clone(),
                                protected_scope: contract.protected_paths.clone(),
                                reason: format!("scope evaluation error: {err}"),
                            }));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Evaluates a shell command line before spawning, checking for substitution and undeclared effects.
    pub fn execute_shell(
        &self,
        command: &str,
        is_verification: bool,
    ) -> Result<(), ExecutionError> {
        let contract = self.contract.as_ref().ok_or_else(|| {
            ExecutionError::ContractUnenforceable(
                "contracted shell execution requires an explicit task contract".into(),
            )
        })?;
        let analysis = crate::shell_policy::analyze_command(command);

        // Command substitution cannot be proven bounded through static allowlists alone.
        if analysis.has_substitution && !self.capabilities.process_isolation {
            return Err(ExecutionError::ShellSubstitutionBlocked(
                "shell command substitution cannot be proven bounded without process isolation"
                    .into(),
            ));
        }

        let declared_effects = analysis.classify_declared_effects();

        // Diagnose a visible network effect at the narrowest missing boundary first.
        if declared_effects.contains(&DeclaredEffect::NetworkAccess) {
            if !self.capabilities.can_enforce_net || !self.capabilities.network_sandbox {
                return Err(ExecutionError::ContractUnenforceable(
                    "shell command attempts network access but backend cannot enforce network sandboxing"
                        .into(),
                ));
            }
            let net_allowed = contract
                .external_effects
                .iter()
                .any(|e| e == "network" || e == "network_access");

            if !net_allowed {
                return Err(ExecutionError::NetworkDisallowed(
                    "shell command contains network operations disallowed by contract".into(),
                ));
            }
        }

        // Every shell command executes project-controlled code. Static command analysis can
        // classify apparent effects, but it cannot confine build scripts, subprocesses, or
        // dynamically loaded code. Refuse unless the selected backend really owns a process
        // isolation boundary.
        // If verification command, verification cannot write outside artifact roots
        if is_verification
            && declared_effects.contains(&DeclaredEffect::FileSystemWrite)
            && analysis.has_redirection
        {
            return Err(ExecutionError::UnauthorizedEffect(
                "verification command contains shell redirection writing outside artifact roots"
                    .into(),
            ));
        }

        if !self.allow_unconfined_shell
            && (!self.capabilities.can_enforce_process || !self.capabilities.process_isolation)
        {
            return Err(ExecutionError::ContractUnenforceable(
                "backend cannot enforce process isolation for contracted shell execution".into(),
            ));
        }

        Ok(())
    }

    /// Validates that an artifact write path stays strictly within allowed artifact write roots.
    pub fn validate_artifact_write(&self, path: &str) -> Result<(), ExecutionError> {
        let contract = self.contract.as_ref().ok_or_else(|| {
            ExecutionError::ContractUnenforceable(
                "artifact validation requires an explicit task contract".into(),
            )
        })?;
        if contract.artifact_write_roots.is_empty() {
            return Err(ExecutionError::ArtifactRootViolation {
                path: path.to_string(),
                allowed_roots: Vec::new(),
            });
        }

        let normalized = resolve_contract_path(&self.trusted_workspace, path).map_err(|err| {
            ExecutionError::Other(format!("artifact path resolution failed: {err}"))
        })?;

        let allowed = contract.artifact_write_roots.iter().any(|root| {
            let norm_root = root.trim_matches('/');
            normalized == norm_root
                || normalized
                    .strip_prefix(norm_root)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        });

        if !allowed {
            return Err(ExecutionError::ArtifactRootViolation {
                path: path.to_string(),
                allowed_roots: contract.artifact_write_roots.clone(),
            });
        }
        Ok(())
    }

    /// Identifies whether a prepared action performs irreversible external effects.
    pub fn is_irreversible(&self, action: &PreparedAction) -> bool {
        action
            .declared_effects
            .contains(&DeclaredEffect::ExternalServicePublish)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::TaskId;

    #[test]
    fn f05_unknown_effects_blocked() {
        assert!(!effect_profile_allows(false, true, true));
        assert!(!effect_profile_allows(true, false, true));
        assert!(!effect_profile_allows(true, true, false));
        assert!(effect_profile_allows(true, true, true));
    }

    #[test]
    fn f05_curl_hidden_in_build_script() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-hidden-build-effects",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let executor = ContractExecutor::new(
            "unconfined-local",
            ExecutorCapabilities::default(),
            temp.path(),
        )
        .with_contract(contract);

        // The command itself contains no network token. Project-controlled build.rs or package
        // hooks may still perform network/filesystem/process effects, so an unconfined backend
        // must refuse before spawning rather than trusting static command inspection.
        let res = executor.execute_shell("cargo build", false);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ContractUnenforceable(msg) => {
                assert!(msg.contains("process isolation"));
            }
            other => panic!("Expected ContractUnenforceable, got: {other:?}"),
        }
    }

    #[test]
    fn f05_shell_substitution() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-shell-substitution",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let executor = ContractExecutor::new(
            "unconfined-local",
            ExecutorCapabilities::default(),
            temp.path(),
        )
        .with_contract(contract);

        // Command substitution $(...) cannot be bounded through static allowlist
        let cmd = "echo $(whoami)";
        let res = executor.execute_shell(cmd, false);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ShellSubstitutionBlocked(msg) => {
                assert!(msg.contains("substitution"));
            }
            other => panic!("Expected ShellSubstitutionBlocked, got: {other:?}"),
        }
    }

    #[test]
    fn f05_test_writes_outside_artifact_roots() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-artifacts",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec!["target/".into(), "artifacts/".into()],
        )
        .unwrap();

        let executor = ContractExecutor::new(
            "test-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);

        // Writing within declared artifact roots is allowed
        assert!(executor
            .validate_artifact_write("target/output.json")
            .is_ok());
        assert!(executor
            .validate_artifact_write("artifacts/report.xml")
            .is_ok());

        // Writing outside declared artifact roots is rejected
        let res = executor.validate_artifact_write("src/codegen.rs");
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ArtifactRootViolation {
                path,
                allowed_roots,
            } => {
                assert_eq!(path, "src/codegen.rs");
                assert!(allowed_roots.contains(&"target/".to_string()));
            }
            other => panic!("Expected ArtifactRootViolation, got: {other:?}"),
        }
    }

    #[test]
    fn f05_missing_sandbox_backend() {
        let temp = tempfile::tempdir().unwrap();
        // Backend cannot enforce process execution
        let caps = ExecutorCapabilities {
            can_enforce_process: false,
            ..Default::default()
        };
        let contract = TaskContract::new(
            "contract-missing-backend",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let executor =
            ContractExecutor::new("weak-backend", caps, temp.path()).with_contract(contract);

        let action = PreparedAction::new("bash", vec![], vec![DeclaredEffect::ProcessExecution]);
        let res = executor.execute(&action);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ContractUnenforceable(msg) => {
                assert!(msg.contains("process isolation"));
            }
            other => panic!("Expected ContractUnenforceable, got: {other:?}"),
        }
    }

    #[test]
    fn f05_network_default_deny() {
        let temp = tempfile::tempdir().unwrap();
        // Backend has full sandbox, but contract does NOT grant network in external_effects
        let contract = TaskContract::new(
            "contract-no-net",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![], // no network grant
            vec![],
            vec![],
        )
        .unwrap();

        let executor = ContractExecutor::new(
            "sandboxed-backend",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);

        let action = PreparedAction::new("web_fetch", vec![], vec![DeclaredEffect::NetworkAccess]);
        let res = executor.execute(&action);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::NetworkDisallowed(msg) => {
                assert!(msg.contains("default deny"));
            }
            other => panic!("Expected NetworkDisallowed, got: {other:?}"),
        }
    }

    #[test]
    fn f05_mcp_read_only_hint_lies() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-mcp-verify",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec!["protected/".into()],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        let executor = ContractExecutor::new(
            "mcp-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);

        // Remote tool advertised readOnlyHint: true, but its action actually mutates protected file
        let deceptive_action = PreparedAction::new(
            "mcp__disk__write",
            vec!["protected/keys.json".into()],
            vec![DeclaredEffect::FileSystemWrite],
        );
        let res = executor.execute(&deceptive_action);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ScopeViolation(v) => {
                assert_eq!(v.requested_target, "protected/keys.json");
            }
            other => panic!("Expected ScopeViolation, got: {other:?}"),
        }
    }

    #[test]
    fn f05_inherited_env_cannot_widen_authority() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-child-bound",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        let executor = ContractExecutor::new(
            "child-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);

        // Inherited env attempts to grant broad mutation outside contracted scope
        let escalated_action = PreparedAction::new(
            "write",
            vec!["config/deploy.yaml".into()],
            vec![DeclaredEffect::FileSystemWrite],
        );
        let res = executor.execute(&escalated_action);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExecutionError::ScopeViolation(v) => {
                assert_eq!(v.requested_target, "config/deploy.yaml");
            }
            other => panic!("Expected ScopeViolation, got: {other:?}"),
        }
    }

    #[test]
    fn f05_artifact_root_requires_path_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-artifact-prefix",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec!["target/".into()],
        )
        .unwrap();
        let executor = ContractExecutor::new(
            "test-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);

        assert!(executor
            .validate_artifact_write("target/report.json")
            .is_ok());
        assert!(executor
            .validate_artifact_write("target2/report.json")
            .is_err());
    }

    #[test]
    fn f05_contracted_executor_never_authorizes_without_contract() {
        let temp = tempfile::tempdir().unwrap();
        let executor = ContractExecutor::new(
            "test-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        );
        let action = PreparedAction::new(
            "write",
            vec!["src/main.rs".into()],
            vec![DeclaredEffect::FileSystemWrite],
        );
        assert!(matches!(
            executor.execute(&action),
            Err(ExecutionError::ContractUnenforceable(_))
        ));
        assert!(matches!(
            executor.execute_shell("cargo test", false),
            Err(ExecutionError::ContractUnenforceable(_))
        ));
    }

    #[test]
    fn f05_verification_artifacts_require_explicit_roots() {
        let temp = tempfile::tempdir().unwrap();
        let contract = TaskContract::new(
            "contract-no-artifact-roots",
            1,
            TaskId::new(),
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec!["cargo test".into()],
            vec![],
        )
        .unwrap();
        let executor = ContractExecutor::new(
            "test-executor",
            ExecutorCapabilities::full_sandbox(),
            temp.path(),
        )
        .with_contract(contract);
        assert!(matches!(
            executor.validate_artifact_write("target/report.json"),
            Err(ExecutionError::ArtifactRootViolation { .. })
        ));
    }
}
