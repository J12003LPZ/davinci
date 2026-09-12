//! Capability run and lifecycle state.

use crate::events::AgentEvent;
use crate::prompt::capabilities::{CapabilityDecision, NativeBehaviorCapability};
use crate::prompt::manifest::hash_text;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRunState {
    pub active: Vec<NativeBehaviorCapability>,
    pub frontend: Option<FrontendDesignState>,
    pub debugging: Option<DebuggingState>,
    pub review: Option<ReviewState>,
    #[serde(skip)]
    in_flight_commands: HashMap<String, String>,
    #[serde(skip)]
    in_flight_edits: HashMap<String, bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendDesignState {
    pub frontend_edit_seen: bool,
    pub visual_backend_available: bool,
    pub visual_snapshot_after_last_edit: bool,
    pub visual_revision_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggingState {
    pub failure_signal_seen: bool,
    pub reproducer_command_digest: Option<String>,
    pub reproducer_failed_before_edit: bool,
    pub causal_edit_seen: bool,
    pub reproducer_passed_after_edit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewState {
    pub review_started: bool,
}

impl CapabilityRunState {
    pub fn reset_for_user_turn(
        &mut self,
        decision: &CapabilityDecision,
        visual_backend_available: bool,
    ) {
        self.active = decision.capabilities.clone();
        self.frontend = decision
            .is_active(NativeBehaviorCapability::FrontendDesign)
            .then(|| FrontendDesignState {
                visual_backend_available,
                ..FrontendDesignState::default()
            });
        self.debugging = decision
            .is_active(NativeBehaviorCapability::Debugging)
            .then(DebuggingState::default);
        self.review = decision
            .is_active(NativeBehaviorCapability::CodeReview)
            .then_some(ReviewState {
                review_started: true,
            });
        self.in_flight_commands.clear();
        self.in_flight_edits.clear();
    }

    pub fn observe_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => self.observe_tool_start(tool_call_id, tool_name, args),
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                is_error,
                ..
            } => self.observe_tool_end(tool_call_id, tool_name, *is_error),
            _ => {}
        }
    }

    fn observe_tool_start(&mut self, tool_call_id: &str, tool_name: &str, args: &Value) {
        if is_edit_tool(tool_name) {
            let after_failure = self
                .debugging
                .as_ref()
                .is_some_and(|debugging| debugging.failure_signal_seen);
            self.in_flight_edits
                .insert(tool_call_id.to_owned(), after_failure);
        }

        if let Some(digest) = command_digest(tool_name, args) {
            self.in_flight_commands
                .insert(tool_call_id.to_owned(), digest);
        }
    }

    fn observe_tool_end(&mut self, tool_call_id: &str, tool_name: &str, is_error: bool) {
        let command_digest = self.in_flight_commands.remove(tool_call_id);
        let edit_after_failure = self.in_flight_edits.remove(tool_call_id);

        if is_edit_tool(tool_name) && !is_error {
            if let Some(frontend) = self.frontend.as_mut() {
                frontend.frontend_edit_seen = true;
                frontend.visual_snapshot_after_last_edit = false;
            }
            if edit_after_failure == Some(true) {
                if let Some(debugging) = self.debugging.as_mut() {
                    debugging.causal_edit_seen = true;
                }
            }
        }

        if tool_name == "visual_snapshot" && !is_error {
            if let Some(frontend) = self.frontend.as_mut() {
                if frontend.visual_backend_available && frontend.frontend_edit_seen {
                    frontend.visual_snapshot_after_last_edit = true;
                    frontend.visual_revision_count =
                        frontend.visual_revision_count.saturating_add(1);
                }
            }
        }

        if let Some(debugging) = self.debugging.as_mut() {
            if is_error {
                debugging.failure_signal_seen = true;
                if !debugging.causal_edit_seen {
                    if let Some(digest) = command_digest {
                        debugging.reproducer_command_digest.get_or_insert(digest);
                        debugging.reproducer_failed_before_edit = true;
                    }
                }
            } else if debugging.causal_edit_seen
                && debugging.reproducer_failed_before_edit
                && command_digest.as_deref() == debugging.reproducer_command_digest.as_deref()
            {
                debugging.reproducer_passed_after_edit = true;
            }
        }
    }
}

fn is_edit_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write" | "edit" | "apply_patch" | "notebook_edit"
    )
}

fn command_digest(tool_name: &str, args: &Value) -> Option<String> {
    if !matches!(tool_name, "bash" | "powershell" | "exec_command") {
        return None;
    }
    args.get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .map(hash_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AgentEvent;
    use crate::prompt::capabilities::{CapabilityDecision, NativeBehaviorCapability};
    use serde_json::json;

    fn decision(capability: NativeBehaviorCapability) -> CapabilityDecision {
        CapabilityDecision {
            capabilities: vec![capability],
            reasons: Vec::new(),
            evidence: Vec::new(),
        }
    }

    fn start(id: &str, name: &str, args: serde_json::Value) -> AgentEvent {
        AgentEvent::ToolExecutionStart {
            tool_call_id: id.to_owned(),
            tool_name: name.to_owned(),
            args,
        }
    }

    fn end(id: &str, name: &str, is_error: bool) -> AgentEvent {
        AgentEvent::ToolExecutionEnd {
            tool_call_id: id.to_owned(),
            tool_name: name.to_owned(),
            result: json!({}),
            is_error,
            details: None,
        }
    }

    #[test]
    fn frontend_transitions_from_edit_to_visual_snapshot() {
        let mut state = CapabilityRunState::default();
        state.reset_for_user_turn(&decision(NativeBehaviorCapability::FrontendDesign), true);

        state.observe_event(&start("edit-1", "edit", json!({"path": "src/App.tsx"})));
        assert!(!state.frontend.as_ref().unwrap().frontend_edit_seen);
        assert!(
            !state
                .frontend
                .as_ref()
                .unwrap()
                .visual_snapshot_after_last_edit
        );

        state.observe_event(&end("edit-1", "edit", false));
        state.observe_event(&start("snapshot-1", "visual_snapshot", json!({})));
        state.observe_event(&end("snapshot-1", "visual_snapshot", false));

        let frontend = state.frontend.as_ref().unwrap();
        assert!(frontend.visual_backend_available);
        assert!(frontend.visual_snapshot_after_last_edit);
        assert_eq!(frontend.visual_revision_count, 1);

        state.observe_event(&start("edit-2", "apply_patch", json!({"patch": "..."})));
        state.observe_event(&end("edit-2", "apply_patch", false));
        let frontend = state.frontend.as_ref().unwrap();
        assert!(!frontend.visual_snapshot_after_last_edit);
        assert_eq!(frontend.visual_revision_count, 1);
    }

    #[test]
    fn debugging_requires_the_failed_reproducer_after_a_causal_edit() {
        let mut state = CapabilityRunState::default();
        state.reset_for_user_turn(&decision(NativeBehaviorCapability::Debugging), false);

        state.observe_event(&start(
            "run-1",
            "bash",
            json!({"command": "cargo test -p davinci-agent"}),
        ));
        state.observe_event(&end("run-1", "bash", true));

        let debugging = state.debugging.as_ref().unwrap();
        assert!(debugging.failure_signal_seen);
        assert!(debugging.reproducer_command_digest.is_some());
        assert!(debugging.reproducer_failed_before_edit);
        assert!(!debugging.causal_edit_seen);

        state.observe_event(&start("edit-1", "edit", json!({"path": "src/lib.rs"})));
        assert!(!state.debugging.as_ref().unwrap().causal_edit_seen);
        state.observe_event(&end("edit-1", "edit", false));
        assert!(state.debugging.as_ref().unwrap().causal_edit_seen);

        state.observe_event(&start(
            "run-2",
            "bash",
            json!({"command": "cargo test -p davinci-agent"}),
        ));
        state.observe_event(&end("run-2", "bash", false));

        assert!(
            state
                .debugging
                .as_ref()
                .unwrap()
                .reproducer_passed_after_edit
        );

        state.reset_for_user_turn(&decision(NativeBehaviorCapability::Debugging), false);
        let debugging = state.debugging.as_ref().unwrap();
        assert!(!debugging.failure_signal_seen);
        assert!(debugging.reproducer_command_digest.is_none());
        assert!(!debugging.reproducer_failed_before_edit);
        assert!(!debugging.causal_edit_seen);
        assert!(!debugging.reproducer_passed_after_edit);
    }

    #[test]
    fn failed_edits_do_not_create_frontend_or_debug_evidence() {
        let mut state = CapabilityRunState::default();
        state.reset_for_user_turn(&decision(NativeBehaviorCapability::FrontendDesign), true);
        state.observe_event(&start("edit-1", "edit", json!({"path": "src/App.tsx"})));
        state.observe_event(&end("edit-1", "edit", true));
        assert!(!state.frontend.as_ref().unwrap().frontend_edit_seen);

        state.reset_for_user_turn(&decision(NativeBehaviorCapability::Debugging), false);
        state.observe_event(&start(
            "run-1",
            "bash",
            json!({"command": "cargo test -p davinci-agent"}),
        ));
        state.observe_event(&end("run-1", "bash", true));
        state.observe_event(&start("edit-1", "edit", json!({"path": "src/lib.rs"})));
        state.observe_event(&end("edit-1", "edit", true));
        state.observe_event(&start(
            "run-2",
            "bash",
            json!({"command": "cargo test -p davinci-agent"}),
        ));
        state.observe_event(&end("run-2", "bash", false));

        let debugging = state.debugging.as_ref().unwrap();
        assert!(!debugging.causal_edit_seen);
        assert!(!debugging.reproducer_passed_after_edit);
    }

    #[test]
    fn exec_command_is_tracked_as_a_reproducer() {
        let mut state = CapabilityRunState::default();
        state.reset_for_user_turn(&decision(NativeBehaviorCapability::Debugging), false);
        let command = "cargo test -p davinci-agent";
        state.observe_event(&start("run-1", "exec_command", json!({"command": command})));
        state.observe_event(&end("run-1", "exec_command", true));
        state.observe_event(&start("edit-1", "edit", json!({"path": "src/lib.rs"})));
        state.observe_event(&end("edit-1", "edit", false));
        state.observe_event(&start("run-2", "exec_command", json!({"command": command})));
        state.observe_event(&end("run-2", "exec_command", false));

        let debugging = state.debugging.as_ref().unwrap();
        assert!(debugging.reproducer_command_digest.is_some());
        assert!(debugging.reproducer_failed_before_edit);
        assert!(debugging.reproducer_passed_after_edit);
    }
}
