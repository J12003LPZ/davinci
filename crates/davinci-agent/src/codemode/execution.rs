use super::*;
use crate::runtime::operations::OperationId;
use crate::{Agent, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

/// Added to a run whose script called tools but returned an empty value.
pub(crate) const EMPTY_RESULT_NOTE: &str = "The script returned an empty value after calling tools. Each tool call resolves to {text, structuredContent, complete}: read `.text` (find, ls and grep give one match per line); a guessed field name such as .matches or .files is undefined.";

/// Preserve lineage and status without restoring text that a safety hook redacted.
pub(crate) fn mandatory_facts(result: &ToolResult) -> Option<Value> {
    let outcome = result.details.as_ref()?.get("codemode")?;
    let children = outcome.get("children")?.as_array()?.iter().take(64).map(|child| {
        json!({"requestId":child.get("requestId"),"ordinal":child.get("ordinal"),
            "tool":child.get("tool"),"operationRef":child.get("operationRef"),"status":child.get("status"),
            "incomplete":child.get("incomplete").cloned().unwrap_or(json!(false))})
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
    /// Keep an admitted host for the turns to come. Hosts attach the operation
    /// journal per turn, after startup, so enabling at startup always failed
    /// with "Codemode requires the parent operation journal".
    pub fn stage_read_only_codemode(&mut self, host: Arc<dyn CodeModeHost>) {
        self.staged_codemode = Some(CodeModeBinding { host });
    }

    pub fn has_staged_codemode(&self) -> bool {
        self.staged_codemode.is_some()
    }

    /// Turn Codemode off: later turns no longer register, advertise, or run it.
    pub fn clear_codemode(&mut self) {
        self.staged_codemode = None;
        self.codemode = None;
        if let Some(runtime) = &self.runtime {
            runtime.capability_registry.unregister("codemode");
        }
        self.tools.retain(|name| name != "codemode");
        self.tool_registry.retain(|name| name != "codemode");
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activate_authorized("codemode", false);
        self.sync_tool_authorization();
    }

    /// Register a staged host on the current runtime. Call after each new
    /// runtime is installed; a runtime that already has Codemode is left alone.
    pub fn activate_staged_codemode(&mut self) -> Result<(), CodeModeError> {
        let Some(binding) = self.staged_codemode.clone() else {
            return Ok(());
        };
        if self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.capability_registry.get("codemode").is_some())
        {
            // The capability survives on this runtime; the binding must still
            // point at the staged host, not a cleared or replaced one.
            self.bind_codemode(binding.host);
            return Ok(());
        }
        self.enable_read_only_codemode(binding.host)
    }

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
        // Advertise the effective ceilings: larger values are clamped by
        // `CodeModeLimits::for_request`, so the model must not be offered them.
        let defaults = CodeModeLimits::default();
        let schema = json!({"type":"object","additionalProperties":false,"required":["code"],
            "properties":{"code":{"type":"string","minLength":1,"maxLength":defaults.script_bytes},
            "timeoutMs":{"type":"integer","minimum":1,"maximum":defaults.wall_ms},
            "maxOutputBytes":{"type":"integer","minimum":4096,"maximum":defaults.output_bytes}}});
        let mut capability = RuntimeCapability::new(
            "codemode",
            CapabilitySource::Builtin,
            crate::permission::ToolClass::Read,
            true,
            &schema,
            Some("1".into()),
        );
        capability.description = "Run one sandboxed JavaScript program over authorized read-only tools and return a small result. Prefer it to several separate read, grep, find or ls calls: one script can list, read, grep, filter and count, and only its return value enters your context. Example: `const [a, b] = await Promise.all([tools.read({path: 'Cargo.toml'}), tools.read({path: 'README.md'})]); return {cargo: a.text.slice(0, 300), readme: b.text.slice(0, 300)};`. Use tools.<name>({...}) to call a tool, codemode.search and codemode.describe to find tools and their schemas. Every tool call resolves to {text, structuredContent, complete}: parse `.text` (find, ls and grep give one match per line) rather than guessing other field names, and if a lookup finds nothing say so instead of returning an empty list. Await every tool call and return plain JSON only: strings, numbers, booleans, null, arrays and plain objects, with no undefined fields (use null), Map, Set, Date or functions. Output past 16 KB is cut, so extract the lines you need instead of returning whole files. Not for a single lookup, edits, or shell commands. Each child result has a `complete` flag: incomplete data must not be treated as complete.".into();
        capability.declared_effects.clear();
        capability.replay_policy = ReplayPolicy::NeverAutoReplay;
        runtime.capability_registry.register(capability);
        self.bind_codemode(host);
        Ok(())
    }

    /// The model can call Codemode this turn: it is bound and authorized, or a
    /// host is staged and registers right after the prompt is prepared (print
    /// mode prepares the first turn's prompt before the runtime exists).
    pub(crate) fn codemode_exposed(&self) -> bool {
        self.staged_codemode.is_some()
            || (self.codemode.is_some()
                && self
                    .tool_context
                    .authorized_tools
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .contains("codemode"))
    }

    fn bind_codemode(&mut self, host: Arc<dyn CodeModeHost>) {
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
    }

    pub(crate) fn run_codemode(
        &self,
        id: &str,
        args: &Value,
        parent: Option<OperationId>,
    ) -> ToolResult {
        let run = || -> Result<(CodeModeOutcome, String), CodeModeError> {
            let request: CodeModeRequest = serde_json::from_value(args.clone())
                .map_err(|_| CodeModeError::new("INVALID_INPUT", "invalid Codemode request"))?;
            let mut limits = CodeModeLimits::default().for_request(&request)?;
            let binding = self
                .codemode
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "Codemode host unavailable"))?;
            let runtime = self
                .runtime
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "parent runtime unavailable"))?;
            if let Some(remaining) = super::remaining_root_wall_ms(runtime.root_budget.as_ref())? {
                limits.wall_ms = limits.wall_ms.min(remaining);
            }
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
            let broker = Arc::new(AgentCodeModeBroker::new(
                leaf_agent,
                context.clone(),
                Some(parent),
            )?);
            let authority = super::authority_fingerprint(self)?;
            let mut outcome = binding.host.execute(&request, &context, broker.clone());
            let drained = broker.close(std::time::Duration::from_millis(
                context.limits.cleanup_grace_ms,
            ));
            // Guest and host output cannot forge the authoritative child evidence.
            outcome.children = broker.children();
            let collisions = broker.alias_collisions();
            if !collisions.is_empty() {
                let shown = collisions.iter().take(16).cloned().collect::<Vec<_>>();
                outcome.host_notes.push(format!(
                    "Tools excluded because their script names collide: {}{}",
                    shown.join(", "),
                    if collisions.len() > shown.len() {
                        ", ..."
                    } else {
                        ""
                    }
                ));
            }
            if !drained {
                outcome
                    .host_notes
                    .push("A child call was still running when the run closed; child evidence may be incomplete.".into());
                if matches!(outcome.status, CodeModeStatus::Completed) {
                    outcome.status = CodeModeStatus::Partial;
                }
            }
            outcome.operation_ref = parent.to_string();
            let incomplete = outcome
                .children
                .iter()
                .filter(|child| child.incomplete)
                .count();
            if incomplete > 0 {
                // Not a failure: a script may page, or not need the rest. But
                // a clean `completed` must not hide that the data was cut.
                outcome.host_notes.push(format!(
                    "{incomplete} child result(s) reached the script truncated or without image content (complete: false); the output may rest on partial data."
                ));
            }
            // A script that called tools and returned nothing usually guessed a
            // field name; say what the real shape is rather than let the model
            // read "[]" as "nothing exists".
            let empty = outcome.script_completed
                && !outcome.children.is_empty()
                && matches!(
                    outcome.output_text.trim(),
                    "" | "[]" | "{}" | "null" | "\"\""
                );
            if empty {
                outcome.host_notes.push(EMPTY_RESULT_NOTE.into());
            }
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
            if empty {
                self.counters
                    .codemode_empty_results
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Ok((outcome, authority))
        };
        // Every terminal outcome counts once, including requests refused
        // before a script ran, so attempts and failures are never undercounted.
        let finish = |completed: bool| {
            use std::sync::atomic::Ordering::Relaxed;
            self.counters.codemode_runs.fetch_add(1, Relaxed);
            if !completed {
                self.counters.codemode_unfinished.fetch_add(1, Relaxed);
            }
        };
        match run() {
            Ok((outcome, authority)) => {
                finish(matches!(outcome.status, CodeModeStatus::Completed));
                ToolResult {
                    content: serde_json::to_string(&outcome)
                        .unwrap_or_else(|_| "Codemode outcome serialization failed".into()),
                    is_error: !matches!(outcome.status, CodeModeStatus::Completed),
                    details: Some(json!({"codemode":outcome,"_codemode_authority":authority})),
                }
            }
            Err(mut error) => {
                finish(false);
                error.operation_ref = parent.map(|parent| parent.to_string());
                let outcome = CodeModeOutcome {
                    status: match error.code.as_str() {
                        "CANCELLED" => CodeModeStatus::Cancelled,
                        "RECOVERY_REQUIRED" => CodeModeStatus::RecoveryRequired,
                        _ => CodeModeStatus::Failed,
                    },
                    script_completed: false,
                    output_text: String::new(),
                    output_complete: false,
                    output_artifact: None,
                    children: vec![],
                    host_notes: vec![],
                    operation_ref: error
                        .operation_ref
                        .clone()
                        .unwrap_or_else(|| "unavailable".into()),
                    error: Some(error.clone()),
                };
                ToolResult {
                    content: serde_json::to_string(&outcome)
                        .unwrap_or_else(|_| error.message.clone()),
                    is_error: true,
                    details: Some(json!({"codemode":outcome,"codemode_error":error})),
                }
            }
        }
    }
}
