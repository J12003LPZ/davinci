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
}
