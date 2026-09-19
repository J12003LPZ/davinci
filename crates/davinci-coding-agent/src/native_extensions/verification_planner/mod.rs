//! Deterministic verification planning.
//!
//! The planner classifies already-observed source paths and returns a bounded
//! list of checks that a caller may execute later.  It never launches a
//! process, mutates the worktree, or reports a check as passed.  Plans carry a
//! source identity so that a later executor can reject stale evidence.

mod model;
mod rules;
mod tools;

#[allow(unused_imports)]
pub use model::{
    VerificationPlan, VerificationPlanArgs, VerificationPlannerConfig, VerificationRequirement,
    VerificationStep, VerificationTelemetry,
};
pub use tools::{tool_spec, TOOL_NAMES};

use davinci_agent::{
    runtime::transactions::{coordinator_for_context, TransactionCoordinator, TransactionOwner},
    PermissionMode, PermissionPolicy, PermissionState, PermissionVerdict, ToolContext, ToolError,
    ToolResult,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Instant,
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct VerificationPlanner {
    root: PathBuf,
    config: VerificationPlannerConfig,
    permissions: Arc<RwLock<Arc<PermissionState>>>,
    cancellation: Arc<RwLock<Option<Arc<AtomicBool>>>>,
    telemetry: Arc<Mutex<VerificationTelemetry>>,
}

impl Default for VerificationPlanner {
    fn default() -> Self {
        Self::new(
            Path::new("."),
            VerificationPlannerConfig {
                enabled: false,
                ..Default::default()
            },
        )
    }
}

impl VerificationPlanner {
    pub fn new(root: &Path, config: VerificationPlannerConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            config: config.bounded(),
            permissions: Arc::new(RwLock::new(Arc::new(PermissionState::new(
                PermissionPolicy::new(PermissionMode::Ask),
            )))),
            cancellation: Arc::new(RwLock::new(None)),
            telemetry: Arc::new(Mutex::new(VerificationTelemetry::default())),
        }
    }

    pub fn set_permissions(&self, permissions: Arc<PermissionState>) {
        *self
            .permissions
            .write()
            .unwrap_or_else(|error| error.into_inner()) = permissions;
    }

    pub fn set_cancellation(&self, signal: Option<Arc<AtomicBool>>) {
        *self
            .cancellation
            .write()
            .unwrap_or_else(|error| error.into_inner()) = signal;
    }

    pub fn status(&self) -> Value {
        let telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        json!({
            "enabled": self.config.enabled,
            "root": self.root,
            "schemaVersion": SCHEMA_VERSION,
            "limits": {
                "maxFiles": self.config.max_files,
                "maxSteps": self.config.max_steps,
                "timeoutMs": self.config.timeout_ms
            },
            "telemetry": telemetry,
            "execution": "planning_only"
        })
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        self.execute_with_context(self.root.as_path(), name, args, None)
    }

    pub fn execute_with_context(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<ToolResult, ToolError> {
        if !TOOL_NAMES.contains(&name) {
            return Err(ToolError::Unknown(name.to_string()));
        }
        let started = Instant::now();
        {
            let mut telemetry = self
                .telemetry
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            telemetry.requests = telemetry.requests.saturating_add(1);
        }

        let result = if !self.config.enabled {
            Ok(self.disabled_result(args))
        } else {
            self.plan(cwd, args, context)
        };

        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.last_latency_ms = latency_ms;
        if result.is_err() {
            telemetry.failures = telemetry.failures.saturating_add(1);
        }
        let telemetry_snapshot = telemetry.clone();
        drop(telemetry);

        result
            .map(|mut value| {
                value["telemetry"] =
                    serde_json::to_value(telemetry_snapshot).unwrap_or_else(|_| json!({}));
                value["telemetry"]["latencyMs"] = json!(latency_ms);
                ToolResult {
                    content: serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|_| value.to_string()),
                    details: Some(value),
                    is_error: false,
                }
            })
            .map_err(ToolError::Failed)
    }

    fn plan(
        &self,
        cwd: &Path,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<Value, String> {
        let request: VerificationPlanArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("verification_plan: invalid arguments: {error}"))?;
        validate_request(&request, &self.config)?;
        self.authorize_tool(cwd, args)?;
        if self.is_cancelled(context) {
            return Ok(self.cancelled_result("planning cancelled before input resolution"));
        }

        let (transaction_files, transaction_identity) =
            self.resolve_transaction(cwd, &request, context)?;
        let paths = transaction_files
            .as_ref()
            .cloned()
            .unwrap_or_else(|| request.files.clone());
        let mut paths = paths;
        if transaction_files.is_none() {
            if let Some(path) = &request.path {
                paths.push(path.clone());
            }
        }
        for path in &paths {
            self.authorize_read(cwd, args, path)?;
        }
        if self.is_cancelled(context) {
            return Ok(self.cancelled_result("planning cancelled after input authorization"));
        }

        let plan = rules::build_plan(
            cwd,
            &request,
            transaction_files,
            transaction_identity,
            &self.config,
        )?;
        serde_json::to_value(plan).map_err(|error| error.to_string())
    }

    fn resolve_transaction(
        &self,
        cwd: &Path,
        request: &VerificationPlanArgs,
        context: Option<&ToolContext>,
    ) -> Result<(Option<Vec<String>>, Option<String>), String> {
        let Some(id) = request.transaction_id.as_deref() else {
            return Ok((None, None));
        };
        if !request.files.is_empty() || request.path.is_some() {
            return Err("transactionId cannot be combined with files or path".into());
        }
        let coordinator = if let Some(context) = context {
            coordinator_for_context(cwd, context).map_err(|error| error.to_string())?
        } else {
            TransactionCoordinator::new(&self.root, TransactionOwner::default())?
        };
        let summary = coordinator.status(id).map_err(|error| {
            format!("transactionId is not owned by this host or is unavailable: {error}")
        })?;
        let mut files = summary.affected_files.clone();
        files.sort();
        files.dedup();
        let identity = digest(
            &serde_json::to_vec(&(
                &summary.workspace_identity,
                &summary.id,
                &summary.state,
                &summary.before_hashes,
                &summary.proposed_hashes,
                &summary.applied_hashes,
            ))
            .map_err(|error| error.to_string())?,
        );
        Ok((Some(files), Some(identity)))
    }

    fn authorize_tool(&self, cwd: &Path, args: &Value) -> Result<(), String> {
        let state = self
            .permissions
            .read()
            .map_err(|_| "permission state unavailable".to_string())?
            .clone();
        let policy = state
            .lock()
            .map_err(|_| "permission policy unavailable".to_string())?;
        match policy.decide("verification-planner", "verification_plan", args, cwd) {
            PermissionVerdict::Allow => Ok(()),
            _ => Err("verification planning denied by current permissions".into()),
        }
    }

    fn authorize_read(&self, cwd: &Path, _args: &Value, path: &str) -> Result<(), String> {
        let state = self
            .permissions
            .read()
            .map_err(|_| "permission state unavailable".to_string())?
            .clone();
        let policy = state
            .lock()
            .map_err(|_| "permission policy unavailable".to_string())?;
        match policy.decide("verification-planner", "read", &json!({"path": path}), cwd) {
            PermissionVerdict::Allow => Ok(()),
            _ => Err(format!("verification planning read denied for {path}")),
        }
    }

    fn is_cancelled(&self, context: Option<&ToolContext>) -> bool {
        context.is_some_and(ToolContext::is_aborted)
            || self
                .cancellation
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .is_some_and(|signal| signal.load(Ordering::Acquire))
    }

    fn disabled_result(&self, args: &Value) -> Value {
        json!({
            "schemaVersion": SCHEMA_VERSION,
            "enabled": false,
            "partial": true,
            "complete": false,
            "input": args,
            "changedFiles": [],
            "requirements": [],
            "steps": [],
            "warnings": ["verification planning is disabled in settings"],
            "execution": "planning_only"
        })
    }

    fn cancelled_result(&self, reason: &str) -> Value {
        json!({
            "schemaVersion": SCHEMA_VERSION,
            "enabled": true,
            "partial": true,
            "complete": false,
            "changedFiles": [],
            "requirements": [],
            "steps": [],
            "warnings": [reason],
            "execution": "planning_only"
        })
    }
}

fn validate_request(
    request: &VerificationPlanArgs,
    config: &VerificationPlannerConfig,
) -> Result<(), String> {
    if request.files.len() > config.max_files {
        return Err(format!(
            "verification_plan: at most {} files are supported",
            config.max_files
        ));
    }
    if request.transaction_id.as_ref().is_some_and(|id| {
        id.is_empty() || id.len() > 128 || id.chars().any(|character| character.is_control())
    }) {
        return Err("verification_plan: invalid transactionId".into());
    }
    for value in request
        .files
        .iter()
        .chain(request.path.iter())
        .chain(request.changed_symbols.iter())
        .chain(request.user_requirements.iter())
    {
        if value.len() > 4096 || value.contains('\0') {
            return Err("verification_plan: argument is oversized or contains NUL".into());
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("sha256:{:x}", digest.finalize())
}
