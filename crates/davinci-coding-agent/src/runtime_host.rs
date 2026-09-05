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
