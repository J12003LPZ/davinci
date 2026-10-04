use super::*;
use crate::{
    runtime::{capacity::WorkerSlotCapacity, operations::OperationId, ConcurrencyPolicy},
    Agent,
};
use serde_json::{json, Value};
use std::sync::Mutex;

struct RunState {
    requests: std::collections::BTreeSet<u32>,
    bytes: usize,
    metadata_calls: u32,
    metadata_bytes: usize,
    terminal: bool,
    terminal_code: &'static str,
    children: Vec<CodeModeChildOutcome>,
}

/// Borrows the actual parent agent. IPC never supplies its identity or policy.
pub struct AgentCodeModeBroker {
    agent: Agent,
    context: CodeModeRunContext,
    parent: Option<OperationId>,
    policy: CapabilityPolicy,
    state: Mutex<RunState>,
    dispatch: WorkerSlotCapacity,
}

impl AgentCodeModeBroker {
    pub fn new(
        agent: Agent,
        context: CodeModeRunContext,
        parent: Option<OperationId>,
    ) -> Result<Self, CodeModeError> {
        let runtime = agent
            .runtime
            .as_ref()
            .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "parent runtime unavailable"))?;
        if runtime.operations.is_none() {
            return Err(CodeModeError::new(
                "UNAVAILABLE",
                "parent operation journal unavailable",
            ));
        }
        let binding = agent
            .cwd
            .canonicalize()
            .map_err(|_| CodeModeError::new("UNAVAILABLE", "workspace binding unavailable"))?
            .to_string_lossy()
            .into_owned();
        if context.identity.workspace_binding != binding
            || context.identity.runtime_run_id != runtime.run_id.to_string()
            || context.identity.parent_operation_ref != parent.map(|id| id.to_string())
            || context
                .root_budget
                .as_ref()
                .map(|budget| budget.binding_identity())
                != runtime
                    .root_budget
                    .as_ref()
                    .map(|budget| budget.binding_identity())
        {
            return Err(CodeModeError::new(
                "DENIED",
                "parent execution binding mismatch",
            ));
        }
        agent.sync_tool_authorization();
        let authorized = agent
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let capabilities = runtime
            .capability_registry
            .list()
            .into_iter()
            .filter(|capability| authorized.contains(&capability.name))
            .collect();
        let policy = CapabilityPolicy::new(context.mode.clone(), capabilities)?;
        if context.capability_revision
            != runtime
                .capability_registry
                .hash_tool_capabilities(&agent.tools)
        {
            return Err(CodeModeError::new(
                "STALE_CAPABILITY",
                "parent capability snapshot changed",
            ));
        }
        Ok(Self {
            agent,
            context,
            parent,
            policy,
            state: Mutex::new(RunState {
                requests: Default::default(),
                bytes: 0,
                metadata_calls: 0,
                metadata_bytes: 0,
                terminal: false,
                terminal_code: "LIMIT_EXCEEDED",
                children: vec![],
            }),
            dispatch: WorkerSlotCapacity::new(),
        })
    }

    fn live(&self) -> Result<(), CodeModeError> {
        super::remaining_root_wall_ms(self.context.root_budget.as_ref())?;
        if self.context.cancellation.is_cancelled() || self.agent.abort_requested() {
            return Err(CodeModeError::new("CANCELLED", "parent run cancelled"));
        }
        let runtime = self
            .agent
            .runtime
            .as_ref()
            .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "runtime unavailable"))?;
        if runtime
            .capability_registry
            .hash_tool_capabilities(&self.agent.tools)
            != self.context.capability_revision
        {
            return Err(CodeModeError::new(
                "STALE_CAPABILITY",
                "capability snapshot changed",
            ));
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.terminal {
            return Err(CodeModeError::new(
                state.terminal_code,
                "run admission is closed",
            ));
        }
        Ok(())
    }

    fn metadata<T: serde::Serialize>(&self, value: T) -> Result<T, CodeModeError> {
        let bytes = super::projection::serialized_size(&value, self.context.limits.metadata_bytes)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.metadata_calls += 1;
        state.metadata_bytes += bytes;
        if state.metadata_calls > self.context.limits.metadata_calls
            || state.metadata_bytes > self.context.limits.metadata_bytes
        {
            state.terminal = true;
            return Err(CodeModeError::new(
                "LIMIT_EXCEEDED",
                "metadata budget exhausted",
            ));
        }
        Ok(value)
    }

    pub fn children(&self) -> Vec<CodeModeChildOutcome> {
        let mut children = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .children
            .clone();
        children.sort_by_key(|child| child.ordinal);
        children
    }

    #[cfg(test)]
    pub(crate) fn admitted_request_count(&self) -> usize {
        self.state.lock().unwrap().requests.len()
    }

    fn authorized_tools(&self) -> std::collections::BTreeSet<String> {
        self.agent.sync_tool_authorization();
        self.agent
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn require_authorized(&self, name: &str) -> Result<(), CodeModeError> {
        if self.authorized_tools().contains(name) {
            Ok(())
        } else {
            Err(CodeModeError::new(
                "DENIED",
                "tool authorization unavailable",
            ))
        }
    }
}

impl CodeModeBroker for AgentCodeModeBroker {
    fn search(&self, query: ToolQuery) -> Result<ToolPage, CodeModeError> {
        self.live()?;
        if query.query.len() > 4096 || query.limit == 0 || query.limit > 20 {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "invalid metadata query",
            ));
        }
        let offset = query
            .cursor
            .as_deref()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| CodeModeError::new("INVALID_INPUT", "invalid metadata cursor"))?;
        let query_text = query.query.to_lowercase();
        let authorized = self.authorized_tools();
        let matched: Vec<_> = self
            .policy
            .tools()
            .into_iter()
            .filter(|capability| authorized.contains(&capability.name))
            .filter(|capability| {
                capability.name.to_lowercase().contains(&query_text)
                    || capability.description.to_lowercase().contains(&query_text)
            })
            .collect();
        if offset > matched.len() {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "metadata cursor exceeds results",
            ));
        }
        let tools = matched
            .iter()
            .skip(offset)
            .take(query.limit)
            .map(|capability| ToolMetadata {
                canonical_name: capability.name.clone(),
                js_name: js_name(&capability.name),
                description: capability.description.chars().take(512).collect(),
                read_only: capability.read_only,
            })
            .collect::<Vec<_>>();
        let next = offset + tools.len();
        self.metadata(ToolPage {
            tools,
            total: matched.len(),
            cursor: (next < matched.len()).then(|| next.to_string()),
        })
    }

    fn describe(&self, canonical_name: &str) -> Result<Value, CodeModeError> {
        self.live()?;
        self.require_authorized(canonical_name)?;
        let capability = self.policy.resolve(canonical_name)?;
        self.metadata(json!({"canonicalName":capability.name,"jsName":js_name(&capability.name),
            "description":capability.description,"inputSchema":capability.schema,"outputSchema":self.agent.tool_context.mcp.output_schema(canonical_name),
            "readOnly":capability.read_only}))
    }

    fn call(&self, call: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
        self.live()?;
        self.require_authorized(&call.tool)?;
        let capability = self.policy.resolve(&call.tool)?;
        let mutating = !capability.read_only;
        let max_concurrency = if matches!(&self.context.mode, CodeModeMode::ReadOnly)
            && capability.concurrency_policy == ConcurrencyPolicy::ParallelSafe
        {
            self.context.limits.parallelism.max(1)
        } else {
            1
        };
        super::projection::validate_json(&call.args, self.context.limits.json_depth)?;
        if !call.args.is_object()
            || super::projection::serialized_size(&call.args, self.context.limits.argument_bytes)
                .is_err()
        {
            return Err(CodeModeError::new(
                "INVALID_INPUT",
                "arguments exceed admitted boundary",
            ));
        }
        let ordinal = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.terminal
                || call.request_id == 0
                || !state.requests.insert(call.request_id)
                || state.requests.len() > self.context.limits.tool_calls as usize
            {
                state.terminal = true;
                return Err(CodeModeError::new(
                    "LIMIT_EXCEEDED",
                    "child request budget exhausted",
                ));
            }
            state.requests.len()
        };
        // A private leaf lane serializes effects without holding parent journal,
        // agent or admission-state locks while a child or approval waits.
        let _lane = self
            .dispatch
            .acquire(max_concurrency, || {
                self.context.cancellation.is_cancelled()
                    || self.agent.abort_requested()
                    || super::remaining_root_wall_ms(self.context.root_budget.as_ref()).is_err()
            })
            .ok_or_else(|| CodeModeError::new("CANCELLED", "queued child cancelled"))?;
        self.live()?;
        let id = format!("{}#codemode:{ordinal}", self.context.identity.invocation_id);
        let (result, structured, operation_ref) = self.agent.dispatch_script_child(
            &self.agent.cwd,
            &id,
            &call.tool,
            &call.args,
            self.parent,
            ordinal,
        );
        let unresolved_effect = if mutating {
            use crate::runtime::operations::{EffectStatus, OperationState};
            self.agent
                .runtime
                .as_ref()
                .and_then(|runtime| runtime.operations.as_ref())
                .and_then(|operations| operations.dispatcher().journal().snapshot().ok())
                .map(|snapshot| {
                    snapshot
                        .attempts
                        .iter()
                        .find(|attempt| attempt.operation_id().to_string() == operation_ref)
                        .map(|attempt| {
                            attempt.state() != OperationState::Succeeded
                                && matches!(
                                    attempt.effect_status(),
                                    EffectStatus::Possible
                                        | EffectStatus::Unknown
                                        | EffectStatus::EffectsObserved
                                )
                        })
                        .unwrap_or(true)
                })
                .unwrap_or(true)
        } else {
            false
        };
        let mut error = (result.is_error || unresolved_effect).then(|| {
            let recovery = unresolved_effect
                || result.details.as_ref().is_some_and(|details| {
                    [
                        "operation_persistence",
                        "ledger_persistence",
                        "replay_blocked",
                    ]
                    .iter()
                    .any(|key| details.get(key) == Some(&Value::Bool(true)))
                });
            let marker = |key| {
                result
                    .details
                    .as_ref()
                    .is_some_and(|details| details.get(key) == Some(&Value::Bool(true)))
            };
            let mut error = CodeModeError::new(
                if recovery {
                    "RECOVERY_REQUIRED"
                } else if marker("denied") || marker("preToolBlocked") {
                    "DENIED"
                } else if marker("cancelled") {
                    "CANCELLED"
                } else if marker("codemode_incomplete") {
                    "INCOMPLETE_DATA"
                } else {
                    "TOOL_FAILED"
                },
                &result.content,
            );
            error.operation_ref = Some(operation_ref.clone());
            error
        });
        let value = if error.is_none() {
            match super::projection::project_script_result(
                result,
                structured,
                operation_ref.clone(),
                self.context.limits.child_result_bytes,
            ) {
                Ok(value) => Some(value),
                Err(mut projection_error) => {
                    projection_error.operation_ref = Some(operation_ref.clone());
                    error = Some(projection_error);
                    None
                }
            }
        } else {
            None
        };
        if let Some(value) = &value {
            let bytes =
                super::projection::serialized_size(value, self.context.limits.child_result_bytes)?;
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.bytes = state.bytes.saturating_add(bytes);
            if state.bytes > self.context.limits.total_result_bytes {
                state.terminal = true;
                let mut budget_error =
                    CodeModeError::new("LIMIT_EXCEEDED", "aggregate child result budget exhausted");
                budget_error.operation_ref = Some(operation_ref.clone());
                error = Some(budget_error);
            }
        }
        if error
            .as_ref()
            .is_some_and(|error| error.code == "RECOVERY_REQUIRED")
        {
            // Uncertain effects close admission even when the guest catches the error.
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.terminal = true;
            state.terminal_code = "RECOVERY_REQUIRED";
        }
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .children
            .push(CodeModeChildOutcome {
                request_id: call.request_id,
                ordinal: ordinal as u32,
                tool: call.tool,
                operation_ref: operation_ref.clone(),
                status: if error
                    .as_ref()
                    .is_some_and(|error| error.code == "RECOVERY_REQUIRED")
                {
                    CodeModeChildStatus::RecoveryRequired
                } else if error.is_some() {
                    match error.as_ref().map(|error| error.code.as_str()) {
                        Some("DENIED") => CodeModeChildStatus::NotStarted,
                        Some("CANCELLED") => CodeModeChildStatus::Cancelled,
                        _ => CodeModeChildStatus::Failed,
                    }
                } else {
                    CodeModeChildStatus::Succeeded
                },
                error: error.clone(),
            });
        if let Some(error) = error {
            return Err(error);
        }
        value.ok_or_else(|| CodeModeError::new("TOOL_FAILED", "child projection unavailable"))
    }
}
