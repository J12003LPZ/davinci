use super::*;
use crate::runtime::operations::OperationId;
use crate::{Agent, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

/// Preserve lineage and status without restoring text that a safety hook redacted.
pub(crate) fn mandatory_facts(result: &ToolResult) -> Option<Value> {
    let outcome = result.details.as_ref()?.get("codemode")?;
    let children = outcome.get("children")?.as_array()?.iter().take(64).map(|child| {
        json!({"requestId":child.get("requestId"),"ordinal":child.get("ordinal"),
            "tool":child.get("tool"),"operationRef":child.get("operationRef"),"status":child.get("status")})
    }).collect::<Vec<_>>();
    Some(
        json!({"status":outcome.get("status"),"scriptCompleted":outcome.get("scriptCompleted"),
        "operationRef":outcome.get("operationRef"),"children":children}),
    )
}

#[derive(Clone)]
pub(crate) struct CodeModeBinding {
    pub host: Arc<dyn CodeModeHost>,
}

impl std::fmt::Debug for CodeModeBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CodeModeBinding(read-only)")
    }
}

impl Agent {
    /// Explicit host-side opt-in. This never resolves or probes a runtime itself.
    /// Controlled mode is deliberately unavailable until its recovery gates pass.
    pub fn enable_read_only_codemode(
        &mut self,
        host: Arc<dyn CodeModeHost>,
    ) -> Result<(), CodeModeError> {
        use crate::runtime::capabilities::{CapabilitySource, ReplayPolicy, RuntimeCapability};
        let runtime = self
            .runtime
            .as_ref()
            .filter(|runtime| runtime.operations.is_some())
            .ok_or_else(|| {
                CodeModeError::new(
                    "UNAVAILABLE",
                    "Codemode requires the parent operation journal",
                )
            })?;
        if runtime.capability_registry.get("codemode").is_some() {
            return Err(CodeModeError::new(
                "UNAVAILABLE",
                "Codemode capability is already registered",
            ));
        }
        let schema = json!({"type":"object","additionalProperties":false,"required":["code"],
            "properties":{"code":{"type":"string","minLength":1,"maxLength":65536},
            "timeoutMs":{"type":"integer","minimum":1,"maximum":300000},
            "maxOutputBytes":{"type":"integer","minimum":4096,"maximum":65536}}});
        let mut capability = RuntimeCapability::new(
            "codemode",
            CapabilitySource::Builtin,
            crate::permission::ToolClass::Read,
            true,
            &schema,
            Some("1".into()),
        );
        capability.description = "Run sandboxed JavaScript with authorized read-only tools. Use tools.<name>({...}), codemode.search, and codemode.describe; return a small result. Incomplete data must not be treated as complete.".into();
        capability.declared_effects.clear();
        capability.replay_policy = ReplayPolicy::NeverAutoReplay;
        runtime.capability_registry.register(capability);
        self.codemode = Some(CodeModeBinding { host });
        self.apply_extension_tools(&["codemode".into()]);
        let authorized = self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains("codemode");
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activate_authorized("codemode", authorized);
        Ok(())
    }

    pub(crate) fn run_codemode(
        &self,
        id: &str,
        args: &Value,
        parent: Option<OperationId>,
    ) -> ToolResult {
        let run = || -> Result<CodeModeOutcome, CodeModeError> {
            let request: CodeModeRequest = serde_json::from_value(args.clone())
                .map_err(|_| CodeModeError::new("INVALID_INPUT", "invalid Codemode request"))?;
            let limits = CodeModeLimits::default().for_request(&request)?;
            let binding = self
                .codemode
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "Codemode host unavailable"))?;
            let runtime = self
                .runtime
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "parent runtime unavailable"))?;
            let parent = parent
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "parent operation unavailable"))?;
            let context = CodeModeRunContext {
                identity: CodeModeIdentity {
                    invocation_id: id.into(),
                    session_id: runtime.session_id.clone(),
                    branch_leaf: None,
                    workspace_binding: self
                        .cwd
                        .canonicalize()
                        .map_err(|_| CodeModeError::new("UNAVAILABLE", "workspace unavailable"))?
                        .to_string_lossy()
                        .into_owned(),
                    runtime_run_id: runtime.run_id.to_string(),
                    parent_operation_ref: Some(parent.to_string()),
                },
                mode: CodeModeMode::ReadOnly,
                limits,
                capability_revision: runtime
                    .capability_registry
                    .hash_tool_capabilities(&self.tools),
                cancellation: runtime.cancellation_token.child_token(),
                root_budget: runtime.root_budget.clone(),
            };
            let mut leaf_agent = self.clone();
            leaf_agent.abort_signal = Some(context.cancellation.as_atomic_bool());
            leaf_agent.runtime = Some(
                runtime
                    .clone()
                    .with_cancellation_token(context.cancellation.clone()),
            );
            let broker = AgentCodeModeBroker::new(&leaf_agent, &context, Some(parent))?;
            let mut outcome = binding.host.execute(&request, &context, &broker);
            // Guest and host output cannot forge the authoritative child evidence.
            outcome.children = broker.children();
            outcome.operation_ref = parent.to_string();
            if outcome
                .children
                .iter()
                .any(|child| matches!(child.status, CodeModeChildStatus::RecoveryRequired))
            {
                outcome.status = CodeModeStatus::RecoveryRequired;
            } else if outcome
                .children
                .iter()
                .any(|child| !matches!(child.status, CodeModeChildStatus::Succeeded))
                && outcome.script_completed
            {
                outcome.status = CodeModeStatus::Partial;
            }
            Ok(outcome)
        };
        match run() {
            Ok(outcome) => ToolResult {
                content: serde_json::to_string(&outcome)
                    .unwrap_or_else(|_| "Codemode outcome serialization failed".into()),
                is_error: !matches!(outcome.status, CodeModeStatus::Completed),
                details: Some(json!({"codemode":outcome})),
            },
            Err(error) => ToolResult {
                content: error.message.clone(),
                is_error: true,
                details: Some(json!({"codemode_error":error})),
            },
        }
    }
}
