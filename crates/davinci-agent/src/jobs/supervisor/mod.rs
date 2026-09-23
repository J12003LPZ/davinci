//! Trusted host process supervision. No model-facing policy or process registry.
//! The caller supplies an authorized, resolved command and retains the owner.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

mod client;
mod helper;
mod platform;
mod wire;
pub use client::Supervisor;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<crate::runtime::operations::ProcessOperationBinding>,
}

impl ProcessConfig {
    pub fn new(
        executable: PathBuf,
        argv: Vec<String>,
        cwd: PathBuf,
        environment: BTreeMap<String, String>,
    ) -> Self {
        Self {
            executable,
            argv,
            cwd,
            environment,
            operation: None,
        }
    }

    pub fn with_operation_binding(
        mut self,
        operation: crate::runtime::operations::ProcessOperationBinding,
    ) -> Self {
        self.operation = Some(operation);
        self
    }

    pub fn with_environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.environment = environment;
        self
    }

    pub fn execution_evidence(
        &self,
        identity: ProcessIdentity,
        launch_state: ProcessLaunchState,
        exit_code: Option<i32>,
        output_complete: Option<bool>,
    ) -> ProcessExecutionEvidence {
        let environment = environment_evidence(&self.environment);
        ProcessExecutionEvidence {
            identity,
            launch_state,
            executable: self.executable.to_string_lossy().into_owned(),
            argv: self.argv.clone(),
            cwd: self.cwd.to_string_lossy().into_owned(),
            environment_references: environment.references,
            environment_digest: environment.digest,
            exit_code,
            output_complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub operation: Option<crate::runtime::operations::ProcessOperationBinding>,
    pub lifetime: uuid::Uuid,
}

impl ProcessIdentity {
    pub(super) fn new(
        operation: Option<crate::runtime::operations::ProcessOperationBinding>,
    ) -> Self {
        Self {
            operation,
            lifetime: uuid::Uuid::new_v4(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLaunchState {
    Started,
    Exited,
    Stopped,
    FailedBeforeChild,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessExecutionEvidence {
    pub identity: ProcessIdentity,
    pub launch_state: ProcessLaunchState,
    pub executable: String,
    pub argv: Vec<String>,
    pub cwd: String,
    /// Environment variable names only; values remain private to the launch pipe.
    pub environment_references: Vec<String>,
    pub environment_digest: String,
    pub exit_code: Option<i32>,
    pub output_complete: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunchError {
    launch_state: ProcessLaunchState,
    identity: ProcessIdentity,
    evidence: ProcessExecutionEvidence,
    message: String,
}

impl ProcessLaunchError {
    pub(super) fn new(
        launch_state: ProcessLaunchState,
        identity: ProcessIdentity,
        config: &ProcessConfig,
        message: impl Into<String>,
    ) -> Self {
        Self {
            launch_state,
            evidence: config.execution_evidence(
                identity.clone(),
                launch_state,
                None,
                match launch_state {
                    ProcessLaunchState::Unknown => None,
                    _ => Some(false),
                },
            ),
            identity,
            message: message.into(),
        }
    }

    pub fn launch_state(&self) -> ProcessLaunchState {
        self.launch_state
    }

    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }

    pub fn evidence(&self) -> &ProcessExecutionEvidence {
        &self.evidence
    }
}

impl std::fmt::Display for ProcessLaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProcessLaunchError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnvironmentEvidence {
    references: Vec<String>,
    digest: String,
}

fn environment_evidence(environment: &BTreeMap<String, String>) -> EnvironmentEvidence {
    use sha2::{Digest, Sha256};

    let mut hash = Sha256::new();
    for (name, value) in environment {
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    EnvironmentEvidence {
        references: environment.keys().cloned().collect(),
        digest: format!("{:x}", hash.finalize()),
    }
}

/// Executable controlled by the host, never by a tool argument.
#[derive(Clone, Debug)]
pub struct SupervisorCommand {
    pub executable: PathBuf,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    pub identity: ProcessIdentity,
    pub launch_state: ProcessLaunchState,
    pub code: Option<i32>,
    pub stopped: bool,
    pub error: Option<String>,
    pub output_complete: bool,
}

#[derive(Debug)]
pub enum ProcessEvent {
    Output(Vec<u8>),
    Finished(ProcessExit),
}

/// Private CLI mode: enters OS ownership before accepting a command on stdin.
pub fn run() -> ! {
    helper::run()
}
