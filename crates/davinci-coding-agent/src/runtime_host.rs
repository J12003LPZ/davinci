//! Host adapters connecting Davinci extensions, hooks, and persistence into the shared runtime.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use davinci_agent::runtime::bus::is_decision_event;
use davinci_agent::{RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope, RuntimeSubscriber};

use crate::hooks::{self, HooksFile};
use davinci_session::default_agent_dir;
use serde_json::Value;

/// Bind the session projections and workflow executor used by the product host.
/// `previous` must come from `Agent::runtime_for_session`, which validates its source.
pub fn configure_session_workflow(
    runtime: davinci_agent::RuntimeHandle,
    session: Option<&davinci_session::JsonlSession>,
    previous: Option<&davinci_agent::RuntimeHandle>,
    store: davinci_agent::WorkflowStateStore,
    runner: Option<davinci_agent::SubagentRunner>,
) -> Result<davinci_agent::RuntimeHandle, String> {
    configure_session_workflow_with_legacy_recovery(
        runtime,
        session,
        previous,
        store,
        runner,
        &HashMap::new(),
    )
}

/// Inspect child executions from the product host boundary.  This delegates
/// to the runtime's existing operation journal rather than creating a second
/// coordinator, so a reopened UI/server/native host sees the same unresolved
/// subagent, workflow, and background-job ownership.
pub fn child_operation_status(
    runtime: &davinci_agent::RuntimeHandle,
) -> Result<Vec<davinci_agent::UnresolvedChild>, String> {
    runtime
        .unresolved_child_operations()
        .map_err(|error| format!("child operation status unavailable: {error}"))
}

/// Configure a session with an explicit host-authored mapping for record-less
/// legacy task creations. Existing journals remain authoritative and ignore
/// the mapping, while missing or unused entries fail closed during migration.
pub fn configure_session_workflow_with_legacy_recovery(
    mut runtime: davinci_agent::RuntimeHandle,
    session: Option<&davinci_session::JsonlSession>,
    previous: Option<&davinci_agent::RuntimeHandle>,
    store: davinci_agent::WorkflowStateStore,
    runner: Option<davinci_agent::SubagentRunner>,
    recovery: &HashMap<davinci_agent::TaskId, davinci_agent::runtime::LegacyTaskRecovery>,
) -> Result<davinci_agent::RuntimeHandle, String> {
    if let Some(previous) = previous {
        runtime = runtime.with_session_state_from(previous);
    }
    if let Some(session) = session {
        runtime = runtime.with_session(&session.header.id);
        if previous.is_none() {
            runtime =
                davinci_agent::runtime::session::restore_session_runtime_with_legacy_recovery(
                    runtime, session, recovery,
                )?;
        }
    }
    let executor = Arc::new(davinci_agent::WorkflowExecutor::new(
        runtime.clone(),
        store,
        runner,
    ));
    Ok(runtime.with_workflow_executor(executor))
}

#[allow(unused_imports)]
pub use crate::completion_delivery::installed_matches;

/// Host adapter bridging runtime task evaluation with budget reservations and delivery proof.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct HostBudgetEvidenceAdapter {
    pub reservation: davinci_agent::runtime::completion::BudgetReservation,
}

#[allow(dead_code)]
impl HostBudgetEvidenceAdapter {
    pub fn new(reservation: davinci_agent::runtime::completion::BudgetReservation) -> Self {
        Self { reservation }
    }

    /// Validates whether an implementation step can proceed without starving verification/handoff.
    pub fn can_dispatch_implementation(&self, estimated_tokens: u64) -> Result<(), String> {
        if self
            .reservation
            .can_reserve_implementation(estimated_tokens)
        {
            Ok(())
        } else {
            Err(
                "Cannot dispatch implementation: verification or handoff reserve would be breached"
                    .into(),
            )
        }
    }

    /// Evaluates a task completion incorporating budget reserves.
    pub fn evaluate_task(
        &self,
        task: &davinci_agent::runtime::tasks::TaskRecord,
        contract: Option<&davinci_agent::runtime::contracts::TaskContract>,
        receipts: &[davinci_agent::runtime::evidence_store::ExecutionReceipt],
        manifest: &davinci_agent::runtime::source_manifest::SourceManifest,
        expected_attempt: u32,
    ) -> davinci_agent::runtime::completion::CompletionEvaluation {
        davinci_agent::runtime::completion::evaluate_task_completion_with_budget(
            task,
            contract,
            receipts,
            manifest,
            expected_attempt,
            Some(&self.reservation),
        )
    }
}

/// Provenance and token estimates for host-provided overhead (system framing, tool schemas, protocol wrapping).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostOverheadProvenance {
    pub system_tokens: u64,
    pub tool_schema_tokens: u64,
    pub wrapper_tokens: u64,
    pub total_overhead_tokens: u64,
    pub source: String,
}

#[allow(dead_code)]
impl HostOverheadProvenance {
    pub fn new(system_tokens: u64, tool_schema_tokens: u64, wrapper_tokens: u64) -> Self {
        let total = system_tokens
            .saturating_add(tool_schema_tokens)
            .saturating_add(wrapper_tokens);
        Self {
            system_tokens,
            tool_schema_tokens,
            wrapper_tokens,
            total_overhead_tokens: total,
            source: "host::runtime_configuration".to_string(),
        }
    }
}

/// RuntimeSubscriber that executes configured lifecycle hooks.
pub struct HooksRuntimeSubscriber {
    hooks: HooksFile,
    config: crate::hooks::HookPolicyConfig,
    cwd: std::path::PathBuf,
    agent_dir: std::path::PathBuf,
    unmet_requirements: Mutex<Vec<String>>,
}

impl HooksRuntimeSubscriber {
    #[allow(dead_code)]
    pub fn new(hooks: HooksFile) -> Self {
        Self::new_with_config(
            hooks,
            crate::hooks::HookPolicyConfig::default(),
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            default_agent_dir(),
        )
    }

    pub fn new_with_config(
        hooks: HooksFile,
        config: crate::hooks::HookPolicyConfig,
        cwd: std::path::PathBuf,
        agent_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            hooks,
            config,
            cwd,
            agent_dir,
            unmet_requirements: Mutex::new(Vec::new()),
        }
    }
}

impl RuntimeSubscriber for HooksRuntimeSubscriber {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        use std::sync::atomic::Ordering;

        if std::env::var("DAVINCI_RUNTIME_HOOKS_V2").as_deref() == Ok("0") {
            return RuntimeDecision::Continue;
        }

        let is_decision = is_decision_event(&event.payload);

        // 1. Check trust and content integrity before running any hooks
        if let Err(err) = self
            .hooks
            .validate_trust_and_integrity(&self.cwd, &self.agent_dir)
        {
            eprintln!("[davinci-hooks] Hook trust or integrity check failed: {err}");
            if is_decision {
                return RuntimeDecision::Deny {
                    reason: format!("hook policy trust or integrity invalidated: {err}"),
                };
            } else {
                return RuntimeDecision::Continue;
            }
        }

        // 2. Check unmet completion requirements on completion proposals
        if matches!(
            &event.payload,
            RuntimeEvent::BeforeCompletion { .. } | RuntimeEvent::TaskCompletionRequested { .. }
        ) {
            let unmet = self
                .unmet_requirements
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !unmet.is_empty() {
                return RuntimeDecision::Deny {
                    reason: format!("unmet completion requirements: {}", unmet.join("; ")),
                };
            }
        }

        let (tool_name, target_path, args, result_str) = match &event.payload {
            RuntimeEvent::PreToolUse { tool, args, .. } => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(std::path::Path::new);
                (tool.as_str(), path, args.clone(), None)
            }
            RuntimeEvent::PostToolUse { tool, is_error, .. } => {
                let res = if *is_error { Some("error") } else { Some("ok") };
                (tool.as_str(), None, serde_json::Value::Null, res)
            }
            RuntimeEvent::PermissionRequested { tool, .. } => {
                (tool.as_str(), None, serde_json::Value::Null, None)
            }
            RuntimeEvent::BeforeWrite { path, bytes } => {
                let args_json = serde_json::json!({ "path": path, "bytes": bytes });
                ("write", Some(path.as_path()), args_json, None)
            }
            RuntimeEvent::AfterWrite {
                path,
                bytes,
                is_error,
            } => {
                let res = if *is_error { Some("error") } else { Some("ok") };
                let args_json = serde_json::json!({ "path": path, "bytes": bytes });
                ("write", Some(path.as_path()), args_json, res)
            }
            RuntimeEvent::BeforeProcessStart {
                executable,
                argv,
                cwd,
            } => {
                let args_json =
                    serde_json::json!({ "executable": executable, "argv": argv, "cwd": cwd });
                ("process_start", Some(cwd.as_path()), args_json, None)
            }
            RuntimeEvent::AfterProcessExit {
                executable,
                argv,
                exit_code,
                is_error,
            } => {
                let res = if *is_error { Some("error") } else { Some("ok") };
                let args_json = serde_json::json!({ "executable": executable, "argv": argv, "exit_code": exit_code });
                ("process_start", None, args_json, res)
            }
            RuntimeEvent::BeforeTest { framework, targets } => {
                let args_json = serde_json::json!({ "framework": framework, "targets": targets });
                ("test", None, args_json, None)
            }
            RuntimeEvent::AfterTest {
                framework,
                targets,
                passed,
                failures,
            } => {
                let res = if *passed { Some("ok") } else { Some("error") };
                let args_json = serde_json::json!({ "framework": framework, "targets": targets, "failures": failures });
                ("test", None, args_json, res)
            }
            RuntimeEvent::BeforeCommit { message, files } => {
                let args_json = serde_json::json!({ "message": message, "files": files });
                ("commit", None, args_json, None)
            }
            RuntimeEvent::AfterCommit {
                commit_id,
                message,
                is_error,
            } => {
                let res = if *is_error { Some("error") } else { Some("ok") };
                let args_json = serde_json::json!({ "commit_id": commit_id, "message": message });
                ("commit", None, args_json, res)
            }
            _ => ("", None, serde_json::Value::Null, None),
        };

        // 3. Dispatch structured rules if hookPolicy is enabled
        if self.config.enabled {
            if let Some(norm_evt) = hooks::rule_event_for(&event.payload) {
                for rule in &self.hooks.rules {
                    if rule.matches(norm_evt, tool_name, target_path) {
                        let timeout = rule.timeout_ms.or(Some(self.config.default_timeout_ms));
                        let max_depth = self.config.max_depth;
                        match hooks::run_rule(
                            rule,
                            norm_evt,
                            tool_name,
                            target_path,
                            &args,
                            result_str,
                            Some(event),
                            timeout,
                            Some(&self.cwd),
                            max_depth,
                        ) {
                            Ok(()) => {}
                            Err(err) => match rule.on_failure {
                                hooks::HookFailurePolicy::Block => {
                                    if is_decision {
                                        return RuntimeDecision::Deny {
                                            reason: format!(
                                                "hook `{}` blocked {norm_evt}: {err}",
                                                rule.action
                                                    .first()
                                                    .map(|s| s.as_str())
                                                    .unwrap_or("")
                                            ),
                                        };
                                    } else {
                                        eprintln!(
                                                "[davinci-hooks] Observe-only hook `{}` with block policy failed: {err}",
                                                rule.action.first().map(|s| s.as_str()).unwrap_or("")
                                            );
                                        let mut unmet = self
                                            .unmet_requirements
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner());
                                        unmet.push(format!(
                                            "hook `{}` failed: {err}",
                                            rule.action.first().map(|s| s.as_str()).unwrap_or("")
                                        ));
                                    }
                                }
                                hooks::HookFailurePolicy::Warn => {
                                    eprintln!(
                                        "[davinci-hooks] Hook `{}` failed (warn): {err}",
                                        rule.action.first().map(|s| s.as_str()).unwrap_or("")
                                    );
                                    hooks::GLOBAL_HOOK_TELEMETRY
                                        .warned
                                        .fetch_add(1, Ordering::Relaxed);
                                }
                                hooks::HookFailurePolicy::Ignore => {
                                    hooks::GLOBAL_HOOK_TELEMETRY
                                        .ignored
                                        .fetch_add(1, Ordering::Relaxed);
                                }
                            },
                        }
                    }
                }
            }
        }

        // 4. Dispatch legacy vectors
        if let Some(kind) = hooks::hook_kind_for(&event.payload) {
            let commands = self.hooks.get_legacy_commands(kind);
            for argv in commands {
                if let Some(reason) =
                    hooks::run_one_envelope(argv, kind, tool_name, &args, result_str, Some(event))
                {
                    if is_decision {
                        return RuntimeDecision::Deny { reason };
                    } else {
                        eprintln!(
                            "[davinci-hooks] Observe-only hook `{}` recorded non-zero status: {reason}",
                            argv.first().map(|s| s.as_str()).unwrap_or("")
                        );
                    }
                }
            }
        }

        RuntimeDecision::Continue
    }
}

/// RuntimeSubscriber that notifies Token Governor and Vector Memory on compaction lifecycle events.
pub struct CompactionRuntimeSubscriber {
    native: Arc<Mutex<crate::native_extensions::NativeExtensionHost>>,
}

impl CompactionRuntimeSubscriber {
    pub fn new(native: Arc<Mutex<crate::native_extensions::NativeExtensionHost>>) -> Self {
        Self { native }
    }
}

impl RuntimeSubscriber for CompactionRuntimeSubscriber {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        if let RuntimeEvent::PostCompact {
            before_tokens,
            after_tokens,
        } = &event.payload
        {
            if before_tokens > after_tokens
                && std::env::var("DAVINCI_RUNTIME_COMPACTION_SUBSCRIBER").as_deref() != Ok("0")
            {
                let mut native = self.native.lock().unwrap_or_else(|e| e.into_inner());
                native.session_compact();
            }
        }
        RuntimeDecision::Continue
    }
}

// Preserve the library's host adapter path; the binary uses shared activation.
#[allow(unused_imports)]
pub use davinci_agent::runtime::session::RuntimeLogSubscriber;

/// Helper to register native vector memory and skill learning context sources into the runtime.
#[allow(dead_code)]
pub fn register_native_context_sources(
    runtime: &davinci_agent::RuntimeHandle,
    memory: Arc<Mutex<crate::native_extensions::VectorMemory>>,
    learning: Arc<Mutex<crate::native_extensions::LearningController>>,
) {
    runtime.register_context_source(Arc::new(
        crate::native_extensions::vector_memory::MemoryContextSource::new(memory),
    ));
    runtime.register_context_source(Arc::new(
        crate::native_extensions::learning::SkillContextSource::new(learning),
    ));
}

/// Helper to register native context sources directly from extension instances.
#[allow(dead_code)]
pub fn register_native_context_sources_from_instances(
    runtime: &davinci_agent::RuntimeHandle,
    memory: &crate::native_extensions::VectorMemory,
    learning: &crate::native_extensions::LearningController,
) {
    runtime.register_context_source(Arc::new(
        crate::native_extensions::vector_memory::MemoryContextSource::from_memory(memory.clone()),
    ));
    runtime.register_context_source(Arc::new(
        crate::native_extensions::learning::SkillContextSource::from_controller(learning.clone()),
    ));
}

/// Universal Token Governor host adapter for virtualizing tool outputs across all runtime agents.
#[derive(Clone)]
#[allow(dead_code)]
pub struct GovernorHostAdapter {
    pub governor: Arc<Mutex<crate::native_extensions::token_governor::TokenGovernor>>,
}

#[allow(dead_code)]
impl GovernorHostAdapter {
    #[allow(dead_code)]
    pub fn new(
        governor: Arc<Mutex<crate::native_extensions::token_governor::TokenGovernor>>,
    ) -> Self {
        Self { governor }
    }

    pub fn from_governor(
        governor: crate::native_extensions::token_governor::TokenGovernor,
    ) -> Self {
        Self {
            governor: Arc::new(Mutex::new(governor)),
        }
    }

    /// Process a tool result after execution, applying compression/virtualization if eligible.
    /// Exempts `memory_search` and `retrieve_output`; large compressible failures remain reversible.
    pub fn process_tool_output(
        &self,
        name: &str,
        args: &serde_json::Value,
        result: davinci_agent::ToolResult,
    ) -> davinci_agent::ToolResult {
        // Recovery and memory tools stay verbatim; error status alone does not bypass reversible compression.
        if name == "memory_search" || name == "retrieve_output" {
            return result;
        }
        let mut gov = match self.governor.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        gov.after_tool(name, args, result)
    }

    /// Retrieve stored output by id or args.
    pub fn retrieve(
        &self,
        args: &serde_json::Value,
    ) -> Result<davinci_agent::ToolResult, davinci_agent::ToolError> {
        let mut gov = match self.governor.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        gov.retrieve(args)
    }

    /// Ensure that if `tools` contains any tool that can produce compressible output,
    /// `retrieve_output` is present in `tools`.
    pub fn ensure_recovery_tool(tools: &mut Vec<String>) {
        crate::native_extensions::token_governor::ensure_governor_recovery_tool(tools);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn f03_host_imports_legacy_tasks_once_and_reconciles_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let path = davinci_session::runtime_log_path(&session.path);
        let mut task =
            davinci_agent::TaskRecord::new(davinci_agent::RunId::new(), "orphaned legacy work");
        task.state = davinci_agent::TaskState::Running;
        task.assigned_to = Some(davinci_agent::AgentId::new());
        task.revision = 9;
        let mut dependent = davinci_agent::TaskRecord::new(davinci_agent::RunId::new(), "waiting");
        dependent.dependencies.push(task.id);
        dependent.state = davinci_agent::TaskState::Pending;
        let events: Vec<_> = [&task, &dependent]
            .into_iter()
            .map(|record| {
                RuntimeEventEnvelope::new(
                    1,
                    record.run_id,
                    None,
                    None,
                    None,
                    RuntimeEvent::TaskCreated {
                        task_id: record.id,
                        record: Some(record.clone()),
                    },
                )
            })
            .collect();
        let bytes = events.iter().fold(String::new(), |mut bytes, event| {
            bytes.push_str(&serde_json::to_string(event).unwrap());
            bytes.push('\n');
            bytes
        });
        std::fs::write(&path, &bytes).unwrap();
        let activate = || {
            configure_session_workflow(
                davinci_agent::RuntimeHandle::new(
                    davinci_agent::RunId::new(),
                    davinci_agent::AgentId::new(),
                    davinci_agent::RuntimeBus::new(),
                ),
                Some(&session),
                None,
                davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
                None,
            )
            .unwrap()
        };
        let runtime = activate();
        let run = runtime.run_id;
        assert_ne!(run, task.run_id);
        assert_eq!(runtime.task_registry.list_tasks(Some(run)).len(), 2);
        let recovered = runtime.task_registry.get_task(&task.id).unwrap();
        assert_eq!(recovered.run_id, task.run_id);
        assert_eq!(recovered.state, davinci_agent::TaskState::Failed);
        assert_eq!(recovered.revision, 10);
        assert_eq!(recovered.result.as_deref(), Some("process_terminated"));
        assert_eq!(
            runtime.task_registry.get_task(&dependent.id).unwrap().state,
            davinci_agent::TaskState::Blocked
        );
        drop(runtime);
        let resumed = activate();
        assert_eq!(resumed.run_id, run);
        assert_eq!(resumed.task_registry.get_task(&task.id), Some(recovered));
        drop(resumed);
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }

    #[test]
    fn f03_host_legacy_tasks_require_migration_without_erasing_history() {
        for empty_journal in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let session =
                davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
            let path = davinci_session::runtime_log_path(&session.path);
            let journal = session.path.with_extension("tasks.jsonl");
            let runtime = davinci_agent::RuntimeHandle::new(
                davinci_agent::RunId::new(),
                davinci_agent::AgentId::new(),
                davinci_agent::RuntimeBus::new(),
            );
            let event = RuntimeEventEnvelope::new(
                1,
                runtime.run_id,
                Some(session.header.id.clone()),
                Some(runtime.agent_id),
                None,
                RuntimeEvent::TaskCreated {
                    task_id: davinci_agent::TaskId::new(),
                    record: None,
                },
            );
            let bytes = format!("{}\n", serde_json::to_string(&event).unwrap());
            std::fs::write(&path, &bytes).unwrap();
            if empty_journal {
                std::fs::write(&journal, []).unwrap();
            }
            let result = configure_session_workflow(
                runtime,
                Some(&session),
                None,
                davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
                None,
            );
            assert!(result.unwrap_err().contains("migration"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
            if empty_journal {
                assert_eq!(std::fs::metadata(&journal).unwrap().len(), 0);
            } else {
                assert!(!journal.exists());
            }
        }
    }

    #[test]
    fn f03_host_legacy_recovery_imports_recordless_tasks_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let path = davinci_session::runtime_log_path(&session.path);
        let task_id = davinci_agent::TaskId::new();
        let run_id = davinci_agent::RunId::new();
        let event = RuntimeEventEnvelope::new(
            1,
            run_id,
            Some(session.header.id.clone()),
            None,
            None,
            RuntimeEvent::TaskCreated {
                task_id,
                record: None,
            },
        );
        let bytes = format!("{}\n", serde_json::to_string(&event).unwrap());
        std::fs::write(&path, &bytes).unwrap();
        let mut recovery = std::collections::HashMap::new();
        recovery.insert(
            task_id,
            davinci_agent::runtime::LegacyTaskRecovery::new("explicitly recovered"),
        );
        let runtime = configure_session_workflow_with_legacy_recovery(
            davinci_agent::RuntimeHandle::new(
                davinci_agent::RunId::new(),
                davinci_agent::AgentId::new(),
                davinci_agent::RuntimeBus::new(),
            ),
            Some(&session),
            None,
            davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
            None,
            &recovery,
        )
        .unwrap();
        let recovered = runtime.task_registry.get_task(&task_id).unwrap();
        assert_eq!(recovered.title, "explicitly recovered");
        assert_eq!(recovered.run_id, run_id);
        assert_eq!(recovered.state, davinci_agent::TaskState::Ready);
        assert_eq!(recovered.assigned_to, None);
        assert_eq!(recovered.result, None);
        assert!(recovered.evidence_refs.is_empty());
        assert!(session.path.with_extension("tasks.jsonl").exists());
        drop(runtime);
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }

    #[test]
    fn f03_host_sessionless_worker_remains_ephemeral() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let worker = || {
            davinci_agent::RuntimeHandle::new(
                davinci_agent::RunId::new(),
                davinci_agent::AgentId::new(),
                davinci_agent::RuntimeBus::new(),
            )
            .with_parent(davinci_agent::AgentId::new())
        };
        let store =
            || davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts"));
        let rejected = configure_session_workflow(worker(), Some(&session), None, store(), None);
        assert!(rejected.unwrap_err().contains("parent coordinator"));
        assert!(!session.path.with_extension("tasks.jsonl").exists());
        let ephemeral = configure_session_workflow(worker(), None, None, store(), None).unwrap();
        assert!(ephemeral.session_id.is_none());
        assert!(ephemeral.task_registry.rehydrate_from_events(&[]).is_ok());
    }

    #[test]
    fn f03_host_resumes_authoritative_task_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let prepare = || {
            configure_session_workflow(
                davinci_agent::RuntimeHandle::new(
                    davinci_agent::RunId::new(),
                    davinci_agent::AgentId::new(),
                    davinci_agent::RuntimeBus::new(),
                ),
                Some(&session),
                None,
                davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
                None,
            )
            .unwrap()
        };
        let first = prepare();
        let run = first.run_id;
        let task = first
            .task_registry
            .create_task(davinci_agent::TaskRecord::new(run, "durable host task"))
            .unwrap();
        first.task_registry.cancel_task(task).unwrap();
        let expected = first.task_registry.get_task(&task).unwrap();
        let orphan = first
            .task_registry
            .create_task(davinci_agent::TaskRecord::new(run, "orphan"))
            .unwrap();
        first
            .task_registry
            .assign_task(orphan, first.agent_id)
            .unwrap();
        let dependent = first
            .task_registry
            .create_task(
                davinci_agent::TaskRecord::new(run, "dependent").with_dependencies(vec![orphan]),
            )
            .unwrap();
        drop(first);
        let resumed = prepare();
        assert_eq!(
            resumed.run_id, run,
            "host must select the saved run before queries"
        );
        assert_eq!(resumed.task_registry.get_task(&task), Some(expected));
        assert_eq!(
            resumed.task_registry.list_tasks(Some(resumed.run_id)).len(),
            3
        );
        let failed = resumed.task_registry.get_task(&orphan).unwrap();
        assert_eq!(failed.state, davinci_agent::TaskState::Failed);
        assert_eq!(failed.result.as_deref(), Some("process_terminated"));
        assert_eq!(
            resumed.task_registry.get_task(&dependent).unwrap().state,
            davinci_agent::TaskState::Blocked
        );
        assert_eq!(
            resumed.workflow_executor.as_ref().unwrap().runtime.run_id,
            run
        );
        drop(resumed);
        assert_eq!(prepare().task_registry.get_task(&orphan), Some(failed));
    }

    use super::*;

    #[test]
    fn f03_runtime_preparation_reports_corrupt_replay() {
        let dir = tempfile::tempdir().unwrap();
        let session = davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let path = davinci_session::runtime_log_path(&session.path);
        let corrupt = b"{broken record}\n{broken record}\n";
        std::fs::write(&path, corrupt).unwrap();
        let result = configure_session_workflow(
            davinci_agent::RuntimeHandle::new(
                davinci_agent::RunId::new(),
                davinci_agent::AgentId::new(),
                davinci_agent::RuntimeBus::new(),
            ),
            Some(&session),
            None,
            davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
            None,
        );
        assert!(
            result.is_err(),
            "corrupt replay must not silently activate an empty runtime"
        );
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn f03_workflow_uses_final_host_session_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut session =
            davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
        session.header.id = "custom-session-id".into();
        let runtime = davinci_agent::RuntimeHandle::new(
            davinci_agent::RunId::new(),
            davinci_agent::AgentId::new(),
            davinci_agent::RuntimeBus::new(),
        );
        let runtime = configure_session_workflow(
            runtime,
            Some(&session),
            None,
            davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
            None,
        )
        .unwrap();
        let executor = runtime.workflow_executor.as_ref().unwrap();
        assert_eq!(executor.runtime.session_id, runtime.session_id);
        assert_eq!(
            executor.runtime.session_id.as_deref(),
            Some("custom-session-id")
        );
        assert_eq!(executor.runtime.run_id, runtime.run_id);
        executor.runtime.emit_observe(RuntimeEvent::PostToolBatch {
            calls: 0,
            failures: 0,
        });
        let events = davinci_session::read_runtime_log::<RuntimeEventEnvelope>(
            &davinci_session::runtime_log_path(&session.path),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id.as_deref(), Some("custom-session-id"));
    }

    #[test]
    fn f03_same_session_turn_retains_tasks_and_lineage() {
        for switch_session in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut agent = davinci_agent::Agent::new("fixture");
            agent.session =
                Some(davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap());
            let journal = agent
                .session
                .as_ref()
                .unwrap()
                .path
                .with_extension("tasks.jsonl");
            let key = serde_json::to_string(&(
                std::fs::canonicalize(&agent.session.as_ref().unwrap().path).unwrap(),
                &agent.session.as_ref().unwrap().header.id,
            ))
            .unwrap();
            let prepare = |agent: &davinci_agent::Agent| {
                let candidate = davinci_agent::RuntimeHandle::new(
                    davinci_agent::RunId::new(),
                    davinci_agent::AgentId::new(),
                    davinci_agent::RuntimeBus::new(),
                );
                configure_session_workflow(
                    candidate,
                    agent.session.as_ref(),
                    agent.runtime_for_session(),
                    davinci_agent::WorkflowStateStore::with_options(
                        1024,
                        dir.path().join("artifacts"),
                    ),
                    None,
                )
                .unwrap()
            };
            agent.set_runtime(prepare(&agent));
            let first = agent.runtime.as_ref().unwrap().clone();
            let task = first
                .task_registry
                .create_task(davinci_agent::TaskRecord::new(
                    first.run_id,
                    "first turn task",
                ))
                .unwrap();
            let worker = davinci_agent::AgentId::new();
            first
                .registry
                .register_agent(davinci_agent::AgentRecord {
                    id: worker,
                    run_id: first.run_id,
                    parent: Some(first.agent_id),
                    kind: davinci_agent::AgentKind::Background,
                    name: "fixture worker".into(),
                    provider: "fixture".into(),
                    model_id: "fixture".into(),
                    cwd: dir.path().into(),
                    state: davinci_agent::AgentState::Running,
                    task_id: Some(task),
                    worktree: None,
                    started_ms: 0,
                    updated_ms: 0,
                    failure_reason: None,
                })
                .unwrap();
            first.task_registry.assign_task(task, worker).unwrap();
            first.send_message(worker, "retained message").unwrap();
            first.cancel();
            first.emit_observe(RuntimeEvent::PostToolBatch {
                calls: 0,
                failures: 0,
            });
            agent.set_runtime(prepare(&agent));
            let second = agent.runtime.as_ref().unwrap();
            assert_eq!(
                second.registry.get(&worker).unwrap().state,
                davinci_agent::AgentState::Running
            );
            assert_eq!(second.agent_id, first.agent_id);
            assert_eq!(second.mailbox.drain(worker, 10).len(), 1);
            assert!(
                first.mailbox.drain(worker, 10).is_empty(),
                "drained messages must not replay next turn"
            );
            assert_eq!(second.run_id, first.run_id);
            assert!(!second.is_cancelled());
            assert_eq!(
                second.task_registry.list_tasks(Some(second.run_id)).len(),
                1
            );
            assert!(second.task_registry.get_task(&task).is_some());
            let added = second
                .task_registry
                .create_task(davinci_agent::TaskRecord::new(
                    second.run_id,
                    "second turn task",
                ))
                .unwrap();
            assert!(
                first.task_registry.get_task(&added).is_some(),
                "registry must be shared, not replayed"
            );
            assert_eq!(
                second.workflow_executor.as_ref().unwrap().runtime.run_id,
                first.run_id
            );
            assert_eq!(
                agent.tool_context.runtime.as_ref().unwrap().run_id,
                first.run_id
            );
            assert!(journal.is_file());
            assert!(davinci_agent::TaskRegistry::open_session_durable(&journal, &key).is_err());
            let log_path = davinci_session::runtime_log_path(&agent.session.as_ref().unwrap().path);
            let before = davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&log_path)
                .unwrap()
                .len();
            second.emit_observe(RuntimeEvent::PostToolBatch {
                calls: 0,
                failures: 0,
            });
            let events =
                davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&log_path).unwrap();
            assert_eq!(events.len(), before + 1);
            let sequences = events
                .iter()
                .filter(|event| matches!(event.payload, RuntimeEvent::PostToolBatch { .. }))
                .map(|event| event.sequence)
                .collect::<Vec<_>>();
            assert_eq!(
                sequences,
                vec![1, 2],
                "same lineage must retain its event counter"
            );
            if switch_session {
                let mut other =
                    davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap();
                other.header.id = agent.session.as_ref().unwrap().header.id.clone();
                agent.session = Some(other);
                let next_session = prepare(&agent);
                assert_ne!(next_session.run_id, first.run_id);
                assert!(next_session.task_registry.list_tasks(None).is_empty());
                assert!(next_session.registry.get(&worker).is_none());
                assert!(next_session.mailbox.drain(worker, 10).is_empty());
            }
        }
    }
    use crate::native_extensions::token_governor::{TokenGovernor, TokenGovernorConfig};
    use davinci_agent::ToolResult;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn f03_background_completion_reaches_current_turn_observers() {
        struct Collect(Arc<Mutex<Vec<RuntimeEventEnvelope>>>);
        impl RuntimeSubscriber for Collect {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                self.0.lock().unwrap().push(event.clone());
                RuntimeDecision::Continue
            }
        }
        let dir = tempdir().unwrap();
        let mut agent = davinci_agent::Agent::new("fixture");
        agent.session =
            Some(davinci_session::JsonlSession::create(dir.path(), "fixture", None).unwrap());
        let prepare = |agent: &davinci_agent::Agent, bus| {
            configure_session_workflow(
                davinci_agent::RuntimeHandle::new(
                    davinci_agent::RunId::new(),
                    davinci_agent::AgentId::new(),
                    bus,
                ),
                agent.session.as_ref(),
                agent.runtime_for_session(),
                davinci_agent::WorkflowStateStore::with_options(1024, dir.path().join("artifacts")),
                None,
            )
            .unwrap()
        };
        agent.set_runtime(prepare(&agent, davinci_agent::RuntimeBus::new()));
        let old = agent.runtime.as_ref().unwrap().clone();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, worker_id) = std::sync::mpsc::channel();
        let wait = Mutex::new(wait);
        let runner = davinci_agent::SubagentRunner::new(move |request| {
            started.send(request.runtime_agent_id.unwrap()).unwrap();
            wait.lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            Ok("fixture complete".into())
        });
        agent.subagent_runner = Some(runner);
        agent.approver = Some(davinci_agent::ToolApprover(Arc::new(|_| {
            davinci_agent::ToolApprovalDecision::AllowOnce
        })));
        let mut first = true;
        agent
            .run_loop(|_| {
                let spawn = std::mem::replace(&mut first, false);
                Ok(davinci_ai::AssistantMessage {
                    id: "fixture".into(),
                    role: "assistant".into(),
                    model: "fixture".into(),
                    usage: None,
                    error_message: None,
                    content: if spawn {
                        vec![davinci_ai::ContentBlock::ToolCall {
                            id: "background-fixture".into(),
                            name: "agent".into(),
                            arguments: json!({"prompt": "fixture", "mode": "background"}),
                        }]
                    } else {
                        vec![davinci_ai::ContentBlock::Text {
                            text: "done".into(),
                        }]
                    },
                    stop_reason: Some(if spawn {
                        davinci_ai::StopReason::ToolUse
                    } else {
                        davinci_ai::StopReason::Stop
                    }),
                })
            })
            .unwrap();
        let worker = worker_id
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let bus = davinci_agent::RuntimeBus::new();
        bus.subscribe(Arc::new(Collect(seen.clone())));
        agent.set_runtime(prepare(&agent, bus));
        release.send(()).unwrap();
        let log = davinci_session::runtime_log_path(&agent.session.as_ref().unwrap().path);
        let completed = |event: &RuntimeEventEnvelope| {
            event.agent_id == Some(worker)
                && matches!(
                    event.payload,
                    RuntimeEvent::AgentStateChanged {
                        to: davinci_agent::AgentState::Completed,
                        ..
                    }
                )
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let events = davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&log).unwrap();
            if events.iter().any(&completed) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background worker did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            seen.lock().unwrap().iter().filter(|e| completed(e)).count(),
            1
        );
        assert_eq!(
            davinci_session::read_runtime_log::<RuntimeEventEnvelope>(&log)
                .unwrap()
                .iter()
                .filter(|e| completed(e))
                .count(),
            1
        );
        assert_eq!(
            agent
                .runtime
                .as_ref()
                .unwrap()
                .registry
                .get(&worker)
                .unwrap()
                .state,
            davinci_agent::AgentState::Completed
        );
        assert_eq!(
            old.registry.get(&worker).unwrap().state,
            davinci_agent::AgentState::Completed
        );
    }

    fn make_adapter() -> (GovernorHostAdapter, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            enabled: true,
            compress_threshold_bytes: 500,
            compress_threshold_lines: 10,
            store_dir: Some(dir.path().to_path_buf()),
            ..TokenGovernorConfig::default()
        };
        let governor = TokenGovernor::new("test-session", config);
        (GovernorHostAdapter::from_governor(governor), dir)
    }

    fn large_content() -> String {
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n".repeat(30)
    }

    #[test]
    fn test_normal_agent_output_virtualization_and_recovery() {
        let (adapter, _dir) = make_adapter();
        let content = large_content();
        assert!(content.len() > 500);

        let res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };

        // Normal agent executes bash
        let processed = adapter.process_tool_output("bash", &json!({"command": "test"}), res);
        assert!(processed.content.contains("Call retrieve_output with id"));
        assert_eq!(
            processed.details.as_ref().unwrap()["tokenGovernor"]["compressed"],
            true
        );

        let output_id = processed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .expect("outputId must be present in details");

        let recovered = adapter
            .retrieve(&json!({"id": output_id}))
            .expect("recovery must succeed");
        assert!(!recovered.is_error);
        assert!(recovered.content.contains("1: Lorem ipsum dolor sit amet"));
        assert!(recovered.content.contains("30: Lorem ipsum dolor sit amet"));
    }

    #[test]
    fn test_subagent_worker_recoverability_and_tool_guarantee() {
        let (adapter, _dir) = make_adapter();
        let mut subagent_tools = vec!["read".to_string(), "grep".to_string(), "bash".to_string()];
        GovernorHostAdapter::ensure_recovery_tool(&mut subagent_tools);
        assert!(
            subagent_tools.contains(&"retrieve_output".to_string()),
            "Subagent with bash must automatically include retrieve_output"
        );

        let content = large_content();
        let res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };

        let processed =
            adapter.process_tool_output("bash", &json!({"command": "subagent_job"}), res);
        assert!(processed.content.contains("Call retrieve_output with id"));
        assert_eq!(
            processed.details.as_ref().unwrap()["tokenGovernor"]["compressed"],
            true
        );

        let output_id = processed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .unwrap();

        let recovered = adapter
            .retrieve(&json!({"id": output_id}))
            .expect("recovery must succeed");
        assert!(!recovered.is_error);
        assert!(recovered.content.contains("1: Lorem ipsum dolor sit amet"));
        assert!(recovered.content.contains("30: Lorem ipsum dolor sit amet"));
    }

    #[test]
    fn test_graph_worker_recoverability() {
        let (adapter, _dir) = make_adapter();
        let graph_tools = crate::native_extensions::graph::roles::role_tools(
            crate::native_extensions::graph::Role::Writer,
        );
        assert!(
            graph_tools.contains(&"retrieve_output".to_string()),
            "Graph writer must include retrieve_output"
        );

        let content = large_content();
        let res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };

        let processed =
            adapter.process_tool_output("bash", &json!({"command": "graph_writer_step"}), res);
        assert!(processed.content.contains("Call retrieve_output with id"));
        assert_eq!(
            processed.details.as_ref().unwrap()["tokenGovernor"]["compressed"],
            true
        );

        let output_id = processed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .unwrap();

        let recovered = adapter
            .retrieve(&json!({"id": output_id}))
            .expect("recovery must succeed");
        assert!(!recovered.is_error);
        assert!(recovered.content.contains("1: Lorem ipsum dolor sit amet"));
        assert!(recovered.content.contains("30: Lorem ipsum dolor sit amet"));
    }

    #[test]
    fn test_workflow_worker_recoverability() {
        let (adapter, _dir) = make_adapter();
        let mut workflow_tools = vec!["bash".to_string(), "read".to_string()];
        GovernorHostAdapter::ensure_recovery_tool(&mut workflow_tools);
        assert!(
            workflow_tools.contains(&"retrieve_output".to_string()),
            "Workflow worker with compressible tools must include retrieve_output"
        );

        let content = large_content();
        let res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };

        let processed =
            adapter.process_tool_output("bash", &json!({"command": "workflow_action"}), res);
        assert!(processed.content.contains("Call retrieve_output with id"));
        assert_eq!(
            processed.details.as_ref().unwrap()["tokenGovernor"]["compressed"],
            true
        );

        let output_id = processed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .unwrap();

        let recovered = adapter
            .retrieve(&json!({"id": output_id}))
            .expect("recovery must succeed");
        assert!(!recovered.is_error);
        assert!(recovered.content.contains("1: Lorem ipsum dolor sit amet"));
        assert!(recovered.content.contains("30: Lorem ipsum dolor sit amet"));
    }

    #[test]
    fn test_exemptions_memory_search_retrieve_output_and_errors() {
        let (adapter, _dir) = make_adapter();
        let content = large_content();

        // 1. Error status is preserved while large output remains reversibly compressed
        let error_res = ToolResult {
            content: content.clone(),
            is_error: true,
            details: None,
        };
        let processed_err = adapter.process_tool_output("bash", &json!({}), error_res);
        assert!(processed_err.is_error);
        assert!(processed_err
            .content
            .contains("Call retrieve_output with id"));
        assert_eq!(
            processed_err.details.as_ref().unwrap()["tokenGovernor"]["compressed"],
            true
        );

        let output_id = processed_err
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .expect("outputId must be present in error details");
        let recovered = adapter
            .retrieve(&json!({"id": output_id}))
            .expect("error recovery must succeed");
        assert!(!recovered.is_error);
        assert!(recovered.content.contains("1: Lorem ipsum dolor sit amet"));
        assert!(recovered.content.contains("30: Lorem ipsum dolor sit amet"));

        // 2. memory_search is exempt
        let mem_res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };
        let processed_mem = adapter.process_tool_output("memory_search", &json!({}), mem_res);
        assert_eq!(processed_mem.content, content);

        // 3. retrieve_output is exempt
        let ret_res = ToolResult {
            content: content.clone(),
            is_error: false,
            details: None,
        };
        let processed_ret = adapter.process_tool_output("retrieve_output", &json!({}), ret_res);
        assert_eq!(processed_ret.content, content);
    }

    #[test]
    fn test_compaction_runtime_subscriber_lifecycle_and_kill_switch() {
        let dir = tempdir().unwrap();
        let native = Arc::new(Mutex::new(
            crate::native_extensions::NativeExtensionHost::new_with_agent_dir(
                "test",
                dir.path(),
                None,
            ),
        ));

        let subscriber = CompactionRuntimeSubscriber::new(native.clone());
        let bus = davinci_agent::RuntimeBus::new();
        bus.subscribe(Arc::new(subscriber));

        let run_id = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();

        // 1. Record a search call in governor
        let args = json!({"query": "target_fn", "path": "src/lib.rs"});
        let _ = native
            .lock()
            .unwrap()
            .governor
            .before_tool("grep", &args, || "head-123".into());
        let res = ToolResult {
            content: "match 1".into(),
            is_error: false,
            details: None,
        };
        let _ = native
            .lock()
            .unwrap()
            .governor
            .after_tool("grep", &args, res);

        // 2. Immediate duplicate call is blocked
        let blocked = native
            .lock()
            .unwrap()
            .governor
            .before_tool("grep", &args, || "head-123".into());
        assert!(
            blocked.is_some(),
            "Duplicate read must be blocked by governor"
        );

        // 3. Emit PreCompact (does not reset ledger)
        let pre_env = davinci_agent::RuntimeEventEnvelope::new(
            1,
            run_id,
            Some("test-session".into()),
            Some(agent_id),
            None,
            RuntimeEvent::PreCompact {
                estimated_tokens: 50_000,
            },
        );
        bus.emit_observe(pre_env);
        assert!(
            native
                .lock()
                .unwrap()
                .governor
                .before_tool("grep", &args, || "head-123".into())
                .is_some(),
            "PreCompact must not clear governor ledgers prematurely"
        );

        // 4. Emit PostCompact event with tokens reduced
        let post_env = davinci_agent::RuntimeEventEnvelope::new(
            2,
            run_id,
            Some("test-session".into()),
            Some(agent_id),
            None,
            RuntimeEvent::PostCompact {
                before_tokens: 50_000,
                after_tokens: 15_000,
            },
        );
        bus.emit_observe(post_env);

        // 5. CompactionRuntimeSubscriber cleared governor ledgers, so duplicate is allowed again
        let allowed = native
            .lock()
            .unwrap()
            .governor
            .before_tool("grep", &args, || "head-123".into());
        assert!(
            allowed.is_none(),
            "PostCompact must clear governor ledgers via CompactionRuntimeSubscriber"
        );
    }

    #[test]
    fn f03_observer_log_does_not_restore_rejected_completion() {
        struct DenyCompletion;
        impl RuntimeSubscriber for DenyCompletion {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCompletionRequested { .. }) {
                    RuntimeDecision::Deny {
                        reason: "fixture rejection".into(),
                    }
                } else {
                    RuntimeDecision::Continue
                }
            }
        }
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("rejected.runtime.jsonl");
        let run_id = davinci_agent::RunId::new();
        let task_id = {
            let bus = davinci_agent::RuntimeBus::new();
            bus.subscribe(Arc::new(RuntimeLogSubscriber::open(&log_path).unwrap()));
            bus.subscribe(Arc::new(DenyCompletion));
            let registry = davinci_agent::TaskRegistry::with_bus(bus);
            let id = registry
                .create_task(davinci_agent::TaskRecord::new(run_id, "rejected"))
                .unwrap();
            assert!(registry
                .complete_task(id, Some("must not persist".into()))
                .is_err());
            id
        };
        let events: Vec<RuntimeEventEnvelope> =
            davinci_session::read_runtime_log(&log_path).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event.payload, RuntimeEvent::TaskCompletionRequested { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event.payload, RuntimeEvent::TaskCompleted { .. })));
        let registry = davinci_agent::TaskRegistry::new();
        registry.rehydrate_from_events(&events).unwrap();
        let restored = registry.get_task(&task_id).unwrap();
        assert_eq!(restored.state, davinci_agent::TaskState::Ready);
        assert_eq!(restored.result, None);
    }

    #[test]
    fn test_runtime_log_subscriber_persists_envelopes() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test_session.runtime.jsonl");

        let subscriber = RuntimeLogSubscriber::open(&log_path).unwrap();
        let bus = davinci_agent::RuntimeBus::new();
        bus.subscribe(Arc::new(subscriber));

        let run_id = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();

        let task_id = davinci_agent::TaskId::new();
        let env1 = davinci_agent::RuntimeEventEnvelope::new(
            1,
            run_id,
            Some("session-xyz".into()),
            Some(agent_id),
            None,
            RuntimeEvent::AgentStateChanged {
                from: davinci_agent::AgentState::Idle,
                to: davinci_agent::AgentState::Running,
            },
        );
        let env2 = davinci_agent::RuntimeEventEnvelope::new(
            2,
            run_id,
            Some("session-xyz".into()),
            Some(agent_id),
            None,
            RuntimeEvent::TaskCreated {
                task_id,
                record: None,
            },
        );

        bus.emit_observe(env1.clone());
        bus.emit_observe(env2.clone());

        // Replay from log sidecar
        let replayed: Vec<davinci_agent::RuntimeEventEnvelope> =
            davinci_session::read_runtime_log(&log_path).unwrap();
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0], env1);
        assert_eq!(replayed[1], env2);
    }

    #[test]
    fn test_crash_fixture_running_worker_rehydrates_as_failed_process_terminated() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("crashed_session.runtime.jsonl");

        let run_id = davinci_agent::RunId::new();
        let worker_id = davinci_agent::AgentId::new();
        let task1_id = davinci_agent::TaskId::new();
        let task2_id = davinci_agent::TaskId::new();

        let worker_rec = davinci_agent::AgentRecord {
            id: worker_id,
            run_id,
            parent: None,
            kind: davinci_agent::AgentKind::WorkflowWorker,
            name: "worker-1".to_string(),
            provider: "mock".to_string(),
            model_id: "mock".to_string(),
            cwd: dir.path().to_path_buf(),
            state: davinci_agent::AgentState::Starting,
            task_id: Some(task1_id),
            worktree: None,
            started_ms: 1000,
            updated_ms: 1000,
            failure_reason: None,
        };

        let mut t1 = davinci_agent::TaskRecord::new(run_id, "T1 (Running at crash)");
        t1.id = task1_id;
        t1.state = davinci_agent::TaskState::Running;
        t1.assigned_to = Some(worker_id);

        let mut t2 = davinci_agent::TaskRecord::new(run_id, "T2 (Dependent)");
        t2.id = task2_id;
        t2.dependencies = vec![task1_id];
        t2.state = davinci_agent::TaskState::Pending;

        // 1. Write runtime log ending abruptly while worker and task1 were Running
        {
            let mut writer = davinci_session::RuntimeLogWriter::open(&log_path).unwrap();
            let env1 = davinci_agent::RuntimeEventEnvelope::new(
                1,
                run_id,
                Some("sess-crash".into()),
                Some(worker_id),
                None,
                davinci_agent::RuntimeEvent::AgentStarted { record: worker_rec },
            );
            let env2 = davinci_agent::RuntimeEventEnvelope::new(
                2,
                run_id,
                Some("sess-crash".into()),
                Some(worker_id),
                None,
                davinci_agent::RuntimeEvent::AgentStateChanged {
                    from: davinci_agent::AgentState::Starting,
                    to: davinci_agent::AgentState::Running,
                },
            );
            let env3 = davinci_agent::RuntimeEventEnvelope::new(
                3,
                run_id,
                Some("sess-crash".into()),
                Some(worker_id),
                None,
                davinci_agent::RuntimeEvent::TaskCreated {
                    task_id: task1_id,
                    record: Some(t1),
                },
            );
            let env4 = davinci_agent::RuntimeEventEnvelope::new(
                4,
                run_id,
                Some("sess-crash".into()),
                None,
                None,
                davinci_agent::RuntimeEvent::TaskCreated {
                    task_id: task2_id,
                    record: Some(t2),
                },
            );

            writer.append(&env1).unwrap();
            writer.append(&env2).unwrap();
            writer.append(&env3).unwrap();
            writer.append(&env4).unwrap();
        }

        // 2. Simulate process resumption: load events and rehydrate fresh RuntimeHandle
        let events: Vec<davinci_agent::RuntimeEventEnvelope> =
            davinci_session::read_runtime_log(&log_path).unwrap();
        assert_eq!(events.len(), 4);

        let bus = davinci_agent::RuntimeBus::new();
        let handle = davinci_agent::RuntimeHandle::new(run_id, davinci_agent::AgentId::new(), bus);
        handle.rehydrate_from_log(&events).unwrap();

        // 3. Worker must be transitioned to Failed with "process_terminated"
        let rehydrated_agent = handle.registry.get(&worker_id).expect("agent exists");
        assert_eq!(rehydrated_agent.state, davinci_agent::AgentState::Failed);
        assert_eq!(
            rehydrated_agent.failure_reason.as_deref(),
            Some("process_terminated")
        );

        // 4. Running task must be Failed with "process_terminated" and dependent cascaded to Blocked
        let rehydrated_t1 = handle
            .task_registry
            .get_task(&task1_id)
            .expect("task1 exists");
        assert_eq!(rehydrated_t1.state, davinci_agent::TaskState::Failed);
        assert_eq!(rehydrated_t1.result.as_deref(), Some("process_terminated"));

        let rehydrated_t2 = handle
            .task_registry
            .get_task(&task2_id)
            .expect("task2 exists");
        assert_eq!(rehydrated_t2.state, davinci_agent::TaskState::Blocked);
    }

    #[test]
    fn test_workflow_resume_fixture_reusing_completed_readonly_phases() {
        let dir = tempdir().unwrap();
        let store = davinci_agent::WorkflowStateStore::with_options(
            32 * 1024,
            dir.path().join("artifacts"),
        );

        let bus = davinci_agent::RuntimeBus::new();
        let run_id = davinci_agent::RunId::new();
        let lead_id = davinci_agent::AgentId::new();
        let handle = davinci_agent::RuntimeHandle::new(run_id, lead_id, bus);

        let executor = davinci_agent::WorkflowExecutor::new(handle, store.clone(), None);

        let wf_id = davinci_agent::WorkflowId::new();
        let spec_json = r#"{
            "schema_version": 1,
            "name": "resumable-readonly-workflow",
            "max_parallel_agents": 2,
            "max_total_agents": 4,
            "phases": [
                {
                    "id": "discover",
                    "join": "all",
                    "workers": [
                        {
                            "id": "discoverer-1",
                            "prompt": "discover files",
                            "tools": ["read", "find"]
                        }
                    ]
                },
                {
                    "id": "analyze",
                    "depends_on": ["discover"],
                    "join": "all",
                    "workers": [
                        {
                            "id": "analyzer-1",
                            "prompt": "analyze findings",
                            "tools": ["read"]
                        }
                    ]
                }
            ]
        }"#;
        let spec: davinci_agent::WorkflowSpec = serde_json::from_str(spec_json).unwrap();

        // 1. Simulate persisted artifact from Phase 1 ("discover")
        let worker_aid = davinci_agent::AgentId::new();
        store
            .put_artifact(
                wf_id,
                "discover",
                worker_aid,
                serde_json::json!({
                    "worker": "discoverer-1",
                    "output": "Found 3 files to inspect",
                }),
                None,
            )
            .unwrap();

        // 2. Resume execution
        let validated_fps = std::collections::HashSet::new();
        let state = executor
            .resume_execution(wf_id, spec, &validated_fps)
            .expect("resume succeeds");

        assert_eq!(state.status, davinci_agent::WorkflowStatus::Completed);
        assert_eq!(
            state.phases["discover"].status,
            davinci_agent::PhaseStatus::Completed
        );
        assert_eq!(
            state.phases["analyze"].status,
            davinci_agent::PhaseStatus::Completed
        );

        // Verify artifacts exist for both phases
        assert_eq!(store.list_phase_artifacts(wf_id, "discover").len(), 1);
        assert_eq!(store.list_phase_artifacts(wf_id, "analyze").len(), 1);
    }

    #[test]
    fn f06_installed_hash_match() {
        assert!(installed_matches(Some("sha-a"), Some("sha-a"), true));
        assert!(!installed_matches(Some("sha-a"), Some("sha-b"), true));
        assert!(!installed_matches(None, None, true));
        assert!(!installed_matches(Some("sha-a"), Some("sha-a"), false));
    }

    #[test]
    fn test_host_budget_evidence_adapter_reserves() {
        let reservation = davinci_agent::runtime::completion::BudgetReservation::new(
            Some(50_000),
            30_000,
            15_000,
            5_000,
            true,
            Some(0.50),
        );
        let adapter = HostBudgetEvidenceAdapter::new(reservation);

        // 30k spent + 15k verify + 5k handoff = 50k ceiling. Available for impl: 0 tokens!
        assert!(adapter.can_dispatch_implementation(0).is_ok());
        assert!(adapter.can_dispatch_implementation(100).is_err());
    }
}
