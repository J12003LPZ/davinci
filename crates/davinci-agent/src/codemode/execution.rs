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
    let outcome_value = result.details.as_ref()?.get("codemode")?;
    let children = outcome_value.get("children")?.as_array()?.iter().take(64).map(|child| {
        json!({"requestId":child.get("requestId"),"ordinal":child.get("ordinal"),
            "tool":child.get("tool"),"operationRef":child.get("operationRef"),"status":child.get("status"),
            "incomplete":child.get("incomplete").cloned().unwrap_or(json!(false))})
    }).collect::<Vec<_>>();
    // The persisted history item is append-only: a later mode switch or model
    // change replays this exact wire identity instead of rewriting it.
    let history = serde_json::from_value::<CodeModeOutcome>(outcome_value.clone())
        .ok()
        .map(|outcome| super::ScriptHistoryItem::from_outcome(&outcome));
    Some(
        json!({"status":outcome_value.get("status"),"scriptCompleted":outcome_value.get("scriptCompleted"),
        "operationRef":outcome_value.get("operationRef"),"children":children,
        "history":history}),
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
            let branch = self.current_branch_ids();
            let lookup = |run_id: &str| {
                registry
                    .get_for(run_id, runtime.session_id.as_deref(), &binding, &branch)
                    .map_err(|error| CodeModeError::new(&error.code, &error.message))
            };
            let mut record = lookup(&request.tool_call_id)?;
            // A foreign owner that died after this process opened the store is
            // resolved here, so Kill reports the real state, not `NOT_OWNER`.
            if !record.state.terminal() && !record.owned_by_this_process() {
                registry.reconcile_if_owner_dead(&record);
                record = lookup(&request.tool_call_id)?;
            }
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
                    // A run that already finished keeps its finished outcome.
                    if !record.state.terminal() {
                        if !record.owned_by_this_process() {
                            return Err(CodeModeError::new(
                                "NOT_OWNER",
                                "script run belongs to another host process; it was not stopped",
                            ));
                        }
                        // The worker records its own terminal state from what it
                        // actually did, so Kill never writes `Cancelled` itself. No
                        // registry lock is held while waiting.
                        let stopped = self.background_runs.cancel(&record.run_id)
                            && self.background_runs.wait_run(
                                &record.run_id,
                                std::time::Duration::from_millis(
                                    CodeModeLimits::default().cleanup_grace_ms,
                                ),
                            );
                        if !stopped {
                            // Either the worker did not quiesce in time, or this
                            // process has no worker for the run (for example its
                            // registration failed). Neither proves the run stopped
                            // or had no effect. A worker that finishes first wins:
                            // the transition is refused once a state is terminal.
                            let _ = registry.transition(
                                &record.run_id,
                                super::ScriptRunState::RecoveryRequired,
                            );
                        }
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
                let mut record = super::ScriptRunRecord::new(
                    &run_id,
                    outer_call_id,
                    runtime.session_id.as_deref(),
                    &workspace,
                    &runtime
                        .capability_registry
                        .hash_tool_capabilities(&self.tools),
                );
                record.branch_origin = self.branch_origin();
                // The run token is a child of the session's: closing or cancelling
                // the session stops every detached Script it owns. The run is
                // admitted before anything is written or spawned, so a shutdown
                // either refuses it here or sees and cancels it.
                let token = runtime.cancellation_token.child_token();
                if !self.background_runs.reserve(&run_id, token.clone()) {
                    return Err(CodeModeError::new(
                        "BACKGROUND_UNAVAILABLE",
                        "the session is closing; no new background Script was started",
                    ));
                }
                if !self.background_runs.track(&registry_dir) {
                    self.background_runs.release(&run_id);
                    return Err(CodeModeError::new(
                        "BACKGROUND_UNAVAILABLE",
                        "the process is shutting down; no new background Script was started",
                    ));
                }
                let admitted = registry
                    .create(&record)
                    .and_then(|()| {
                        // The handle is visible as running before the thread works.
                        registry
                            .transition(&run_id, super::ScriptRunState::Running)
                            .map(|_| ())
                    })
                    .map_err(|error| {
                        self.background_runs.release(&run_id);
                        CodeModeError::new(&error.code, &error.message)
                    });
                admitted?;
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
                        detached_worker(&registry_dir, &spawned_run, || {
                            let result =
                                agent.run_script(&spawned_run, &request, None, thread_token, true);
                            let _ = agent.finish_script_run(|| result, None);
                        });
                    })
                    .map_err(|_| {
                        self.background_runs.release(&run_id);
                        // No worker ever ran, so the visible handle must not stay open.
                        let _ = registry.transition(&run_id, super::ScriptRunState::Failed);
                        CodeModeError::new("UNAVAILABLE", "background Script worker unavailable")
                    })?;
                self.background_runs.attach(&run_id, handle);
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
    ///
    /// New runs are refused while the drain is in progress, so a launch racing
    /// it cannot escape. Safe to call more than once, including concurrently:
    /// a second call finds nothing left or reports the same unfinished runs.
    /// No registry lock is held while waiting.
    pub fn shutdown_background_scripts(&self, grace: std::time::Duration) -> Vec<String> {
        self.background_runs.close();
        self.background_runs.cancel_all();
        let unfinished = self.background_runs.wait_finished(grace);
        self.background_runs.reopen();
        if let Some(evidence) = self.evidence.as_ref() {
            super::background::mark_unresolved(&evidence.dir().join("script-runs"), &unfinished);
        }
        unfinished
    }

    /// `shutdown_background_scripts` with the Script cleanup grace, for hosts
    /// closing or replacing a session.
    pub fn shutdown_session_scripts(&self) -> Vec<String> {
        self.shutdown_background_scripts(std::time::Duration::from_millis(
            CodeModeLimits::default().cleanup_grace_ms,
        ))
    }

    /// Entry ids on the path from the session root to the current leaf.
    fn current_branch_ids(&self) -> std::collections::HashSet<String> {
        self.session
            .as_ref()
            .map(|session| {
                davinci_session::branch_entries(&session.entries, session.leaf_id.as_deref())
                    .into_iter()
                    .map(|entry| entry.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The leaf a run starting now belongs to.
    fn branch_origin(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|session| session.leaf_id.clone())
    }

    fn script_wakeup_registry(&self) -> Option<(super::ScriptRunRegistry, String)> {
        let evidence = self.evidence.as_ref()?;
        let workspace = self.cwd.canonicalize().ok()?.to_string_lossy().into_owned();
        let registry = super::ScriptRunRegistry::open(evidence.dir().join("script-runs")).ok()?;
        Some((registry, workspace))
    }

    /// Finished runs whose wakeup has not been delivered, oldest first. Reading
    /// does not mark anything delivered: call `acknowledge_script_wakeup` after
    /// the wakeup is durable in the session.
    pub fn pending_script_wakeups(&self) -> Vec<super::ScriptWakeup> {
        let (Some((registry, workspace)), Some(runtime)) =
            (self.script_wakeup_registry(), self.runtime.as_ref())
        else {
            return Vec::new();
        };
        registry
            .pending_notifications(
                runtime.session_id.as_deref(),
                &workspace,
                &self.current_branch_ids(),
            )
            .unwrap_or_default()
            .iter()
            .map(super::ScriptWakeup::from_record)
            .collect()
    }

    /// Mark one wakeup delivered. Returns false when the store refused, so the
    /// caller knows the wakeup may be offered again.
    pub fn acknowledge_script_wakeup(&self, run_id: &str) -> bool {
        self.script_wakeup_registry()
            .is_some_and(|(registry, _)| registry.mark_notified(run_id).is_ok())
    }

    pub(crate) fn run_codemode(
        &self,
        id: &str,
        args: &Value,
        parent: Option<OperationId>,
    ) -> ToolResult {
        let request: CodeModeRequest = match CodeModeRequest::from_tool_arguments(args) {
            Some(request) => request,
            None => {
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
            self.run_script(id, &request, parent, cancellation, false)
        };
        self.finish_script_run(run, parent)
    }

    /// Run one script to completion with the given cancellation authority.
    /// `parent` stays optional so a detached run needs no turn operation.
    /// A `detached` run publishes a bounded summary with its terminal state.
    fn run_script(
        &self,
        id: &str,
        request: &CodeModeRequest,
        parent: Option<OperationId>,
        cancellation: crate::runtime::CancellationToken,
        detached: bool,
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
            let authority = super::authority_fingerprint(self)?;
            // Write-ahead the run before the guest starts, so a crash leaves a
            // durable record to reconcile instead of an invisible orphan.
            let runs = self.evidence.as_ref().and_then(|evidence| {
                super::ScriptRunRegistry::open(evidence.dir().join("script-runs")).ok()
            });
            if let Some(registry) = &runs {
                let mut record = super::ScriptRunRecord::new(
                    &context.identity.invocation_id,
                    id,
                    context.identity.session_id.as_deref(),
                    &context.identity.workspace_binding,
                    &context.capability_revision,
                );
                record.branch_origin = self.branch_origin();
                if registry.create(&record).is_ok() {
                    let _ = registry.transition(&record.run_id, super::ScriptRunState::Running);
                }
            }
            let mut log_ref = None;
            let mut outcome = binding.host.execute(request, &context, broker.clone());
            let drained = broker.close(std::time::Duration::from_millis(
                context.limits.cleanup_grace_ms,
            ));
            // Guest and host output cannot forge the authoritative child evidence.
            outcome.children = broker.children();
            // Capture the full pre-truncation body before anything else mutates
            // the outcome. The host measured the cut; this layer owns the store.
            let session = context
                .identity
                .session_id
                .as_deref()
                .unwrap_or("sessionless");
            if let Some(body) = outcome.retained_output.take() {
                let omitted_bytes = outcome.omitted_bytes;
                let omitted_line = outcome.omitted_start_line.unwrap_or(1);
                if let Some(evidence) = self.evidence.as_ref() {
                    match evidence.store_in_session(session, "script-output", &body) {
                        Ok(path) => {
                            outcome.output_artifact = Some(super::CodeModeArtifact {
                                id: path.to_string_lossy().into_owned(),
                                kind: super::CodeModeArtifactKind::ToolOutput,
                                bytes: body.len() as u64,
                            });
                            outcome.host_notes.push(format!(
                                "Full output retained at {}; {} bytes omitted; the omitted region begins at line {}; call read with path: {}, offset: {}, limit: 2000.",
                                path.display(),
                                omitted_bytes,
                                omitted_line,
                                path.display(),
                                omitted_line
                            ));
                        }
                        Err(_) => outcome.host_notes.push(format!(
                            "Full output not retained: evidence store write failed; {} bytes omitted beginning at line {}.",
                            omitted_bytes, omitted_line
                        )),
                    }
                } else {
                    outcome.host_notes.push(format!(
                        "Full output not retained: no evidence store in this agent; {} bytes omitted beginning at line {}; rerun with a narrower script or smaller output.",
                        omitted_bytes, omitted_line
                    ));
                }
            }
            if let Some(evidence) = self.evidence.as_ref() {
                let events = script_log_events(&outcome);
                if let Ok(log) = super::ScriptLogStore::new(evidence.dir(), session)
                    .persist_script_log(&context.identity.invocation_id, &events)
                {
                    log_ref = Some(log.id.clone());
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
            if !drained {
                // A child may still be making effects: no other status is true.
                outcome.status = CodeModeStatus::RecoveryRequired;
            }
            if empty {
                self.counters
                    .codemode_empty_results
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // Only the final status is published: the registry never records a
            // terminal state that the evaluation above then contradicts.
            if let Some(registry) = &runs {
                let state = match outcome.status {
                    CodeModeStatus::Completed | CodeModeStatus::Partial => {
                        super::ScriptRunState::Completed
                    }
                    CodeModeStatus::Failed => super::ScriptRunState::Failed,
                    CodeModeStatus::Cancelled => super::ScriptRunState::Cancelled,
                    CodeModeStatus::RecoveryRequired => super::ScriptRunState::RecoveryRequired,
                };
                // A detached run's summary is written in the same locked
                // operation, so a terminal record always has it.
                let summary = detached.then(|| {
                    (
                        super::ScriptRunSummary::bounded(
                            &format!("{:?}", outcome.status),
                            &outcome.host_notes,
                        ),
                        log_ref,
                    )
                });
                let _ = registry.finish(&context.identity.invocation_id, state, summary);
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
                    retained_output: None,
                    omitted_bytes: 0,
                    omitted_start_line: None,
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

/// Run a detached Script worker body. A panic leaves effects unsettled, and
/// this live process can never be probed dead, so it is published as
/// `RecoveryRequired`. The panic payload may hold untrusted text and is
/// dropped. Without a panic, a run the body left open becomes `Stalled`.
fn detached_worker(registry_dir: &std::path::Path, run_id: &str, work: impl FnOnce()) {
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err();
    let Ok(registry) = super::ScriptRunRegistry::open(registry_dir) else {
        return;
    };
    if panicked {
        let summary = super::ScriptRunSummary::bounded(
            "recoveryRequired",
            &["The background Script worker panicked; its effects may be unsettled.".to_string()],
        );
        let _ = registry.finish(
            run_id,
            super::ScriptRunState::RecoveryRequired,
            Some((summary, None)),
        );
    } else if registry
        .get(run_id)
        .is_ok_and(|record| !record.state.terminal())
    {
        // `run_script` already set the terminal state; this is a no-op unless
        // it failed to, in which case the run must not look successful.
        let _ = registry.transition(run_id, super::ScriptRunState::Stalled);
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_panicking_worker_leaves_a_recovery_required_wakeup() {
        let dir = tempfile::tempdir().unwrap();
        let registry = super::super::ScriptRunRegistry::open(dir.path()).unwrap();
        registry
            .create(&ScriptRunRecord::new("run", "call", Some("s"), "w", "rev"))
            .unwrap();
        registry.transition("run", ScriptRunState::Running).unwrap();

        detached_worker(dir.path(), "run", || panic!("secret payload"));

        let record = registry.get("run").unwrap();
        assert_eq!(record.state, ScriptRunState::RecoveryRequired);
        let summary = record.summary.expect("summary stored");
        assert!(!summary.notifications.concat().contains("secret payload"));
        let pending = registry
            .pending_notifications(Some("s"), "w", &HashSet::new())
            .unwrap();
        assert_eq!(pending.len(), 1);
    }
}
