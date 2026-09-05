//! Host adapters connecting Davinci extensions, hooks, and persistence into the shared runtime.

use std::sync::{Arc, Mutex};

use davinci_agent::runtime::bus::is_decision_event;
use davinci_agent::{RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope, RuntimeSubscriber};

use crate::hooks::{self, HooksFile};

/// RuntimeSubscriber that executes configured lifecycle hooks.
pub struct HooksRuntimeSubscriber {
    hooks: HooksFile,
}

impl HooksRuntimeSubscriber {
    pub fn new(hooks: HooksFile) -> Self {
        Self { hooks }
    }
}

impl RuntimeSubscriber for HooksRuntimeSubscriber {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        if std::env::var("DAVINCI_RUNTIME_HOOKS_V2").as_deref() == Ok("0") {
            return RuntimeDecision::Continue;
        }

        let kind = match hooks::hook_kind_for(&event.payload) {
            Some(k) => k,
            None => return RuntimeDecision::Continue,
        };

        let tool_name = match &event.payload {
            RuntimeEvent::PreToolUse { tool, .. } => tool.as_str(),
            RuntimeEvent::PostToolUse { tool, .. } => tool.as_str(),
            RuntimeEvent::PermissionRequested { tool, .. } => tool.as_str(),
            _ => "",
        };

        let args = match &event.payload {
            RuntimeEvent::PreToolUse { args, .. } => args,
            _ => &serde_json::Value::Null,
        };

        let result_str = match &event.payload {
            RuntimeEvent::PostToolUse { is_error, .. } => {
                if *is_error {
                    Some("error")
                } else {
                    Some("ok")
                }
            }
            _ => None,
        };

        let commands: Vec<&Vec<String>> = match kind {
            "sessionStart" => self.hooks.session_start.iter().collect(),
            "userPromptSubmit" => self.hooks.user_prompt_submit.iter().collect(),
            "preTool" => self.hooks.pre_tool.iter().collect(),
            "permissionRequest" => self.hooks.permission_request.iter().collect(),
            "postTool" => self.hooks.post_tool.iter().collect(),
            "postToolFailure" => self
                .hooks
                .post_tool_failure
                .iter()
                .chain(self.hooks.post_tool.iter())
                .collect(),
            "postToolBatch" => self.hooks.post_tool_batch.iter().collect(),
            "subagentStart" => self.hooks.subagent_start.iter().collect(),
            "subagentStop" => self.hooks.subagent_stop.iter().collect(),
            "taskCreated" => self.hooks.task_created.iter().collect(),
            "taskCompleted" => self.hooks.task_completed.iter().collect(),
            "preCompact" => self.hooks.pre_compact.iter().collect(),
            "postCompact" => self.hooks.post_compact.iter().collect(),
            "preModelSwitch" => self.hooks.pre_model_switch.iter().collect(),
            "postModelSwitch" => self.hooks.post_model_switch.iter().collect(),
            "sessionEnd" => self
                .hooks
                .session_end
                .iter()
                .chain(self.hooks.stop.iter())
                .collect(),
            _ => Vec::new(),
        };

        let is_decision = is_decision_event(&event.payload);

        for argv in commands {
            if let Some(reason) =
                hooks::run_one_envelope(argv, kind, tool_name, args, result_str, Some(event))
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

/// RuntimeSubscriber that appends runtime lifecycle events into the session's `.runtime.jsonl` sidecar.
pub struct RuntimeLogSubscriber {
    writer: Arc<Mutex<davinci_session::RuntimeLogWriter>>,
}

impl RuntimeLogSubscriber {
    pub fn new(writer: davinci_session::RuntimeLogWriter) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    pub fn open(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, davinci_session::RuntimeLogError> {
        let writer = davinci_session::RuntimeLogWriter::open(path)?;
        Ok(Self::new(writer))
    }
}

impl RuntimeSubscriber for RuntimeLogSubscriber {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.append(event);
        }
        RuntimeDecision::Continue
    }
}

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
    /// Exempts `memory_search`, `retrieve_output`, and error outputs.
    pub fn process_tool_output(
        &self,
        name: &str,
        args: &serde_json::Value,
        result: davinci_agent::ToolResult,
    ) -> davinci_agent::ToolResult {
        // memory_search, retrieve_output, and error outputs are strictly exempt
        if result.is_error || name == "memory_search" || name == "retrieve_output" {
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
        let gov = match self.governor.lock() {
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
    use super::*;
    use crate::native_extensions::token_governor::{TokenGovernor, TokenGovernorConfig};
    use davinci_agent::ToolResult;
    use serde_json::json;
    use tempfile::tempdir;

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

        // 1. Error output is exempt
        let error_res = ToolResult {
            content: content.clone(),
            is_error: true,
            details: None,
        };
        let processed_err = adapter.process_tool_output("bash", &json!({}), error_res);
        assert_eq!(processed_err.content, content);
        assert!(processed_err.is_error);

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
}
