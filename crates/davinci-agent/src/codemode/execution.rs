use super::*;
use crate::runtime::operations::OperationId;
use crate::{Agent, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

/// Build the retained, bounded evidence of one run: status, the model-visible
/// output and one record per child. Full file reads are not retained.
fn script_log_events(outcome: &CodeModeOutcome) -> Vec<super::ScriptLogEvent> {
    let mut events = vec![super::ScriptLogEvent::status(format!(
        "{:?} (scriptCompleted={})",
        outcome.status, outcome.script_completed
    ))];
    if !outcome.output_text.is_empty() {
        events.push(super::ScriptLogEvent::output(outcome.output_text.clone()));
    }
    for child in &outcome.children {
        let summary = format!(
            "{} request {} ordinal {} {:?}",
            child.tool, child.request_id, child.ordinal, child.status
        );
        let detail = child
            .error
            .as_ref()
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| child.operation_ref.clone());
        events.push(super::ScriptLogEvent::child(
            child.tool.clone(),
            summary,
            detail,
        ));
    }
    events
}

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
    pub mode: CodeModeMode,
}

impl std::fmt::Debug for CodeModeBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "CodeModeBinding({:?})", self.mode)
    }
}

impl Agent {
    /// Keep an admitted host for the turns to come. Hosts attach the operation
    /// journal per turn, after startup, so enabling at startup always failed
    /// with "Codemode requires the parent operation journal".
    pub fn stage_read_only_codemode(&mut self, host: Arc<dyn CodeModeHost>) {
        self.staged_codemode = Some(CodeModeBinding {
            host,
            mode: CodeModeMode::ReadOnly,
        });
    }

    pub fn has_staged_codemode(&self) -> bool {
        self.staged_codemode.is_some()
    }

    /// Host-owned opt-in for `off | read-only | controlled`. The mode is
    /// resolved from trusted settings only, never from script source.
    pub fn enable_codemode(
        &mut self,
        host: Arc<dyn CodeModeHost>,
        mode: CodeModeMode,
    ) -> Result<(), CodeModeError> {
        match mode {
            CodeModeMode::Off => {
                self.clear_codemode();
                Ok(())
            }
            CodeModeMode::Controlled if !host.admits_controlled_mode() => Err(CodeModeError::new(
                "DENIED",
                "controlled Codemode is not admitted by this host",
            )),
            _ => self.register_codemode(host, mode),
        }
    }

    /// Stage a host for the turns to come without a runtime yet.
    pub fn stage_codemode(
        &mut self,
        host: Arc<dyn CodeModeHost>,
        mode: CodeModeMode,
    ) -> Result<(), CodeModeError> {
        match mode {
            CodeModeMode::Off => {
                self.clear_codemode();
                Ok(())
            }
            CodeModeMode::Controlled if !host.admits_controlled_mode() => Err(CodeModeError::new(
                "DENIED",
                "controlled Codemode is not admitted by this host",
            )),
            _ => {
                self.staged_codemode = Some(CodeModeBinding { host, mode });
                Ok(())
            }
        }
    }

    /// Turn Codemode off: later turns no longer register, advertise, or run it.
    pub fn clear_codemode(&mut self) {
        self.staged_codemode = None;
        self.codemode = None;
        if let Some(runtime) = &self.runtime {
            runtime.capability_registry.unregister("codemode");
            runtime.capability_registry.unregister("wait_for_script");
        }
        self.tools
            .retain(|name| name != "codemode" && name != "wait_for_script");
        self.tool_registry
            .retain(|name| name != "codemode" && name != "wait_for_script");
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activate_authorized("codemode", false);
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .activate_authorized("wait_for_script", false);
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
            self.bind_codemode(binding.host, binding.mode);
            return Ok(());
        }
        self.register_codemode(binding.host, binding.mode)
    }

    /// Explicit host-side opt-in. This never resolves or probes a runtime itself.
    /// Controlled mode is admitted only when the host declares it and the
    /// recovery gates have passed.
    pub fn enable_read_only_codemode(
        &mut self,
        host: Arc<dyn CodeModeHost>,
    ) -> Result<(), CodeModeError> {
        self.enable_codemode(host, CodeModeMode::ReadOnly)
    }

    fn register_codemode(
        &mut self,
        host: Arc<dyn CodeModeHost>,
        mode: CodeModeMode,
    ) -> Result<(), CodeModeError> {
        use crate::runtime::capabilities::{CapabilitySource, ReplayPolicy, RuntimeCapability};
        if matches!(mode, CodeModeMode::Off) {
            return Err(CodeModeError::new("DENIED", "Codemode mode is off"));
        }
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
        let controlled = matches!(mode, CodeModeMode::Controlled);
        let mut capability = RuntimeCapability::new(
            "codemode",
            CapabilitySource::Builtin,
            if controlled {
                crate::permission::ToolClass::Shell
            } else {
                crate::permission::ToolClass::Read
            },
            !controlled,
            &schema,
            Some("1".into()),
        );
        capability.description = if controlled {
            "Run one sandboxed JavaScript program over authorized tools, including guarded file edits and shell commands. Prefer it to several separate calls: list, read, grep, patch, run the tests and filter the output inside one program, so only its return value enters your context. Use tools.<name>({...}) to call a tool, codemode.search and codemode.describe to find tools and their schemas. Every tool call resolves to {text, structuredContent, complete}: parse `.text` rather than guessing other field names. Await every tool call and return plain JSON only. A successful edit is durable even if the script later throws. Output past 16 KB is cut, so extract the lines you need instead of returning whole files.".into()
        } else {
            "Run one sandboxed JavaScript program over authorized read-only tools and return a small result. Prefer it to several separate read, grep, find or ls calls: one script can list, read, grep, filter and count, and only its return value enters your context. Example: `const [a, b] = await Promise.all([tools.read({path: 'Cargo.toml'}), tools.read({path: 'README.md'})]); return {cargo: a.text.slice(0, 300), readme: b.text.slice(0, 300)};`. Use tools.<name>({...}) to call a tool, codemode.search and codemode.describe to find tools and their schemas. Every tool call resolves to {text, structuredContent, complete}: parse `.text` (find, ls and grep give one match per line) rather than guessing other field names, and if a lookup finds nothing say so instead of returning an empty list. Await every tool call and return plain JSON only: strings, numbers, booleans, null, arrays and plain objects, with no undefined fields (use null), Map, Set, Date or functions. Output past 16 KB is cut, so extract the lines you need instead of returning whole files. Not for a single lookup, edits, or shell commands. Each child result has a `complete` flag: incomplete data must not be treated as complete.".into()
        };
        capability.declared_effects.clear();
        if controlled {
            capability.declared_effects = vec![
                crate::runtime::DeclaredEffect::FileSystemWrite,
                crate::runtime::DeclaredEffect::ProcessExecution,
            ];
        }
        capability.replay_policy = ReplayPolicy::NeverAutoReplay;
        runtime.capability_registry.register(capability);
        if controlled {
            // `wait_for_script` is the only way the model polls a durable run.
            // It is host-owned, session-scoped and only offered in controlled
            // mode; it grants no authority over the run itself.
            let mut wait = RuntimeCapability::new(
                "wait_for_script",
                CapabilitySource::Builtin,
                crate::permission::ToolClass::Read,
                false,
                &json!({"type":"object","additionalProperties":false,
                    "required":["toolCallId","action"],
                    "properties":{"toolCallId":{"type":"string","minLength":1,"maxLength":128},
                    "action":{"type":"string","enum":["status","wait","kill"]}}}),
                Some("1".into()),
            );
            wait.description = "Check, wait for or stop a Script run started earlier in this session. `status` is nonblocking and idempotent, `wait` is bounded and cancellation-aware, and `kill` reports the observed state or RECOVERY_REQUIRED when ownership is not confirmed. Handles are scoped to this session and workspace.".into();
            wait.declared_effects.clear();
            wait.replay_policy = ReplayPolicy::NeverAutoReplay;
            runtime.capability_registry.register(wait);
        }
        self.bind_codemode(host, mode);
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

    fn bind_codemode(&mut self, host: Arc<dyn CodeModeHost>, mode: CodeModeMode) {
        let controlled = matches!(mode, CodeModeMode::Controlled);
        self.codemode = Some(CodeModeBinding { host, mode });
        let mut names = vec!["codemode".to_string()];
        if controlled {
            names.push("wait_for_script".into());
        }
        self.apply_extension_tools(&names);
        let authorized = self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let mut exposure = self
            .tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        exposure.activate_authorized("codemode", authorized.contains("codemode"));
        if controlled {
            exposure.activate_authorized("wait_for_script", authorized.contains("wait_for_script"));
        }
        drop(exposure);
    }

    /// Host-resolved status/wait/kill for a durable Script run. The handle is
    /// opaque and only resolves for the session and workspace that own it.
    pub fn wait_for_script(&self, request: WaitForScriptRequest) -> ToolResult {
        let result = (|| -> Result<serde_json::Value, CodeModeError> {
            let evidence = self
                .evidence
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "script run store unavailable"))?;
            let runtime = self
                .runtime
                .as_ref()
                .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "runtime unavailable"))?;
            let registry = super::ScriptRunRegistry::open(evidence.dir().join("script-runs"))
                .map_err(|error| CodeModeError::new("UNAVAILABLE", &error.message))?;
            let binding = self
                .cwd
                .canonicalize()
                .map_err(|_| CodeModeError::new("UNAVAILABLE", "workspace unavailable"))?
                .to_string_lossy()
                .into_owned();
            let lookup = |run_id: &str| {
                registry
                    .get_for(run_id, runtime.session_id.as_deref(), &binding)
                    .map_err(|error| CodeModeError::new(&error.code, &error.message))
            };
            let mut record = lookup(&request.tool_call_id)?;
            match request.action {
                super::WaitAction::Status => {}
                super::WaitAction::Wait => {
                    // Bounded and cancellation-aware: never block past the root
                    // deadline, and never claim a state the record does not have.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(250);
                    while !record.state.terminal() && std::time::Instant::now() < deadline {
                        if self.abort_requested() {
                            return Err(CodeModeError::new("CANCELLED", "wait cancelled"));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        record = lookup(&request.tool_call_id)?;
                    }
                }
                super::WaitAction::Kill => {
                    if record.state == super::ScriptRunState::RecoveryRequired {
                        return Err(CodeModeError::new(
                            "RECOVERY_REQUIRED",
                            "script run has an unresolved effect; ownership is not confirmed",
                        ));
                    }
                    if record.state == super::ScriptRunState::Running {
                        registry
                            .transition(&record.run_id, super::ScriptRunState::Cancelled)
                            .map_err(|error| CodeModeError::new(&error.code, &error.message))?;
                        record = lookup(&request.tool_call_id)?;
                    }
                }
            }
            Ok(serde_json::json!({
                "toolCallId": record.run_id,
                "state": record.state,
                "terminal": record.state.terminal(),
            }))
        })();
        match result {
            Ok(value) => ToolResult {
                content: value.to_string(),
                is_error: false,
                details: Some(value),
            },
            Err(error) => ToolResult {
                content: serde_json::json!({"error": error}).to_string(),
                is_error: true,
                details: None,
            },
        }
    }

    /// Controlled Script is bound for this run.
    pub fn codemode_controlled(&self) -> bool {
        self.codemode.as_ref().is_some_and(|binding| {
            matches!(binding.mode, crate::codemode::CodeModeMode::Controlled)
        }) || self.staged_codemode.as_ref().is_some_and(|binding| {
            matches!(binding.mode, crate::codemode::CodeModeMode::Controlled)
        })
    }

    /// Public, mode-gated Script reference for hosts and tests. It is `None`
    /// unless controlled Script is the selected mode and an alias is bound.
    pub fn script_mode_prompt_text(&self) -> Option<String> {
        self.script_mode_prompt().map(|module| module.body)
    }

    /// The Script prompt module for this run, built from the same alias
    /// snapshot the broker dispatches against.
    pub(crate) fn script_mode_prompt(&self) -> Option<crate::prompt::PromptModule> {
        let binding = self.codemode.as_ref()?;
        if !matches!(binding.mode, CodeModeMode::Controlled) {
            return None;
        }
        let runtime = self.runtime.as_ref()?;
        let authorized = self
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
        let policy = super::CapabilityPolicy::new(binding.mode.clone(), capabilities).ok()?;
        crate::prompt::script_mode_module(&binding.mode, &policy.script_aliases())
    }

    /// Start a detached run and return its opaque handle immediately. The run
    /// is owned by this agent: closing the session cancels it, and a host that
    /// cannot stop it must treat the run as unresolved.
    fn start_background_script(
        &self,
        outer_call_id: &str,
        request: &CodeModeRequest,
    ) -> ToolResult {
        let start =
            || -> Result<serde_json::Value, CodeModeError> {
                let binding = self.codemode.as_ref().ok_or_else(|| {
                    CodeModeError::new("UNAVAILABLE", "Codemode host unavailable")
                })?;
                if !binding.host.admits_background_scripts() {
                    return Err(CodeModeError::new(
                        "BACKGROUND_UNAVAILABLE",
                        "background Scripts are not admitted by this host",
                    ));
                }
                // Reject an impossible request before a run id exists.
                CodeModeLimits::default().for_request(request)?;
                let evidence = self.evidence.as_ref().ok_or_else(|| {
                    CodeModeError::new(
                        "UNAVAILABLE",
                        "background Scripts need the durable run store",
                    )
                })?;
                let runtime = self.runtime.as_ref().ok_or_else(|| {
                    CodeModeError::new("UNAVAILABLE", "parent runtime unavailable")
                })?;
                let workspace = self
                    .cwd
                    .canonicalize()
                    .map_err(|_| CodeModeError::new("UNAVAILABLE", "workspace unavailable"))?
                    .to_string_lossy()
                    .into_owned();
                let registry_dir = evidence.dir().join("script-runs");
                let registry = super::ScriptRunRegistry::open(&registry_dir)
                    .map_err(|error| CodeModeError::new(&error.code, &error.message))?;
                let outer: String = outer_call_id
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .take(40)
                    .collect();
                let run_id = format!(
                    "bg-{outer}-{}",
                    &uuid::Uuid::new_v4().simple().to_string()[..12]
                );
                let record = super::ScriptRunRecord::new(
                    &run_id,
                    outer_call_id,
                    runtime.session_id.as_deref(),
                    &workspace,
                    &runtime
                        .capability_registry
                        .hash_tool_capabilities(&self.tools),
                );
                registry
                    .create(&record)
                    .map_err(|error| CodeModeError::new(&error.code, &error.message))?;
                // The handle is visible as running before the thread does any work.
                registry
                    .transition(&run_id, super::ScriptRunState::Running)
                    .map_err(|error| CodeModeError::new(&error.code, &error.message))?;

                // The run token is a child of the session's: closing or cancelling
                // the session stops every detached Script it owns.
                let token = runtime.cancellation_token.child_token();
                let agent = self.clone();
                let request = request.clone();
                let spawned_run = run_id.clone();
                let thread_token = token.clone();
                let registry_dir = registry_dir.clone();
                let handle = std::thread::Builder::new()
                    .name(format!(
                        "codemode-{}",
                        &spawned_run[spawned_run.len().saturating_sub(8)..]
                    ))
                    .spawn(move || {
                        // The run's own counters and terminal state are recorded by
                        // `run_script`; a detached failure never escapes the thread.
                        let result = agent.run_script(&spawned_run, &request, None, thread_token);
                        let _ = agent.finish_script_run(|| result, None);
                        if let Ok(registry) = super::ScriptRunRegistry::open(&registry_dir) {
                            // `run_script` already set the terminal state; this is a
                            // no-op unless it failed to, in which case the run must
                            // not look like it finished successfully.
                            if registry
                                .get(&spawned_run)
                                .is_ok_and(|record| !record.state.terminal())
                            {
                                let _ = registry
                                    .transition(&spawned_run, super::ScriptRunState::Stalled);
                            }
                        }
                    })
                    .map_err(|_| {
                        CodeModeError::new("UNAVAILABLE", "background Script worker unavailable")
                    })?;
                self.background_runs.register(&run_id, token, handle);
                Ok(json!({"toolCallId": run_id, "state": "running"}))
            };
        match start() {
            Ok(value) => ToolResult {
                content: value.to_string(),
                is_error: false,
                details: Some(value),
            },
            Err(error) => self.script_error_result(None, error),
        }
    }

    /// Stop every owned detached run. Returns the ids that did not finish in
    /// the grace period; those are recorded as `recoveryRequired`, never as
    /// successful or cancelled.
    pub fn shutdown_background_scripts(&self, grace: std::time::Duration) -> Vec<String> {
        self.background_runs.cancel_all();
        let unfinished = self.background_runs.wait_finished(grace);
        if let Some(evidence) = self.evidence.as_ref() {
            if let Ok(registry) = super::ScriptRunRegistry::open(evidence.dir().join("script-runs"))
            {
                for run_id in &unfinished {
                    let _ = registry.transition(run_id, super::ScriptRunState::RecoveryRequired);
                }
            }
        }
        unfinished
    }

    /// The oldest finished run whose wakeup has not been delivered, marked
    /// delivered exactly once.
    pub fn take_script_notification(&self) -> Option<serde_json::Value> {
        let evidence = self.evidence.as_ref()?;
        let runtime = self.runtime.as_ref()?;
        let workspace = self.cwd.canonicalize().ok()?.to_string_lossy().into_owned();
        let registry = super::ScriptRunRegistry::open(evidence.dir().join("script-runs")).ok()?;
        let pending = registry
            .pending_notifications(runtime.session_id.as_deref(), &workspace)
            .ok()?
            .into_iter()
            .next()?;
        registry.mark_notified(&pending.run_id).ok()?;
        Some(json!({
            "toolCallId": pending.run_id,
            "state": pending.state,
            "logRef": pending.log_ref,
        }))
    }

    pub(crate) fn run_codemode(
        &self,
        id: &str,
        args: &Value,
        parent: Option<OperationId>,
    ) -> ToolResult {
        let request: CodeModeRequest = match serde_json::from_value(args.clone()) {
            Ok(request) => request,
            Err(_) => {
                let error = CodeModeError::new("INVALID_INPUT", "invalid Codemode request");
                return self.script_error_result(parent, error);
            }
        };
        // A background directive detaches the run instead of holding the turn.
        if super::script_requests_background(&request.code) {
            return self.start_background_script(id, &request);
        }
        let cancellation = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.cancellation_token.child_token())
            .unwrap_or_default();
        let run = || -> Result<(CodeModeOutcome, String), CodeModeError> {
            self.run_script(id, &request, parent, cancellation)
        };
        self.finish_script_run(run, parent)
    }

    /// Run one script to completion with the given cancellation authority.
    /// `parent` stays optional so a detached run needs no turn operation.
    fn run_script(
        &self,
        id: &str,
        request: &CodeModeRequest,
        parent: Option<OperationId>,
        cancellation: crate::runtime::CancellationToken,
    ) -> Result<(CodeModeOutcome, String), CodeModeError> {
        {
            let mut limits = CodeModeLimits::default().for_request(request)?;
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
            let parent_operation_ref = parent.map(|parent| parent.to_string());
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
                    parent_operation_ref,
                },
                mode: binding.mode.clone(),
                limits,
                capability_revision: runtime
                    .capability_registry
                    .hash_tool_capabilities(&self.tools),
                cancellation,
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
                parent,
            )?);
            // Write-ahead the run before the guest starts, so a crash leaves a
            // durable record to reconcile instead of an invisible orphan.
            let runs = self.evidence.as_ref().and_then(|evidence| {
                super::ScriptRunRegistry::open(evidence.dir().join("script-runs")).ok()
            });
            if let Some(registry) = &runs {
                let record = super::ScriptRunRecord::new(
                    &context.identity.invocation_id,
                    id,
                    context.identity.session_id.as_deref(),
                    &context.identity.workspace_binding,
                    &context.capability_revision,
                );
                if registry.create(&record).is_ok() {
                    let _ = registry.transition(&record.run_id, super::ScriptRunState::Running);
                }
            }
            let authority = super::authority_fingerprint(self)?;
            let mut outcome = binding.host.execute(request, &context, broker.clone());
            if let Some(registry) = &runs {
                let state = match outcome.status {
                    CodeModeStatus::Completed | CodeModeStatus::Partial => {
                        super::ScriptRunState::Completed
                    }
                    CodeModeStatus::Failed => super::ScriptRunState::Failed,
                    CodeModeStatus::Cancelled => super::ScriptRunState::Cancelled,
                    CodeModeStatus::RecoveryRequired => super::ScriptRunState::RecoveryRequired,
                };
                let _ = registry.transition(&context.identity.invocation_id, state);
            }
            let drained = broker.close(std::time::Duration::from_millis(
                context.limits.cleanup_grace_ms,
            ));
            // Guest and host output cannot forge the authoritative child evidence.
            outcome.children = broker.children();
            if let Some(evidence) = self.evidence.as_ref() {
                let session = context
                    .identity
                    .session_id
                    .clone()
                    .unwrap_or_else(|| "sessionless".into());
                let events = script_log_events(&outcome);
                if let Ok(log) = super::ScriptLogStore::new(evidence.dir(), session)
                    .persist_script_log(&context.identity.invocation_id, &events)
                {
                    outcome.host_notes.push(format!(
                        "Script log {} retained {} event(s), {} bytes{}.",
                        log.id,
                        log.events,
                        log.bytes,
                        if log.truncated { " (truncated)" } else { "" }
                    ));
                }
            }
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
            outcome.operation_ref = parent
                .map(|parent| parent.to_string())
                .unwrap_or_else(|| format!("ephemeral:{id}"));
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
        }
    }

    /// Count every terminal outcome once and shape the tool result, including
    /// requests refused before a script ran, so attempts are never undercounted.
    fn finish_script_run(
        &self,
        run: impl FnOnce() -> Result<(CodeModeOutcome, String), CodeModeError>,
        parent: Option<OperationId>,
    ) -> ToolResult {
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

    fn script_error_result(&self, parent: Option<OperationId>, error: CodeModeError) -> ToolResult {
        self.finish_script_run(|| Err(error), parent)
    }
}
