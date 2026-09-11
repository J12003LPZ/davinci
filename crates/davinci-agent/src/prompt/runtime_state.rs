//! Typed runtime prompt state suffix.

use crate::permission::PermissionMode;
use crate::prompt::composer::{PromptCacheClass, PromptModule};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePromptState {
    pub permission_mode: PermissionMode,
    pub plan_revision: Option<u64>,
    pub plan_approved: bool,
    pub active_contract: bool,
    #[serde(default)]
    pub visual_verification_available: bool,
}

pub fn runtime_state_text(state: &RuntimePromptState) -> String {
    let mode_desc = match state.permission_mode {
        PermissionMode::ReadOnly => {
            "Permission mode: Plan Mode (read-only).\nThe harness will refuse mutations in this mode. Research, inspect, and refine the plan."
        }
        PermissionMode::Ask => {
            "Permission mode: Ask.\nMutating actions and external commands require user confirmation before execution."
        }
        PermissionMode::Edits => {
            "Permission mode: Edits.\nFile editing tools are pre-approved; shell commands and high-risk actions require confirmation."
        }
        PermissionMode::Auto => {
            "Permission mode: Auto.\nStandard coding tools run autonomously within configured workspace boundaries."
        }
        PermissionMode::AlwaysApprove => {
            "Permission mode: AlwaysApprove.\nTool requests run with auto-approval, but runtime boundaries, explicit denies, and filesystem security checks still strictly apply."
        }
    };

    let mut lines = vec![mode_desc.to_string()];

    if let Some(rev) = state.plan_revision {
        if state.plan_approved {
            lines.push(format!("Active plan revision: {}; approved.", rev));
        } else {
            lines.push(format!("Active plan revision: {}; not yet approved.", rev));
        }
    }

    if state.active_contract {
        lines.push("Active task contract: in effect.".to_string());
    }

    lines.push(format!(
        "Visual verification backend: {}.",
        if state.visual_verification_available {
            "available"
        } else {
            "unavailable"
        }
    ));

    format!("<runtime_state>\n{}\n</runtime_state>", lines.join("\n"))
}

pub fn runtime_state_module(state: &RuntimePromptState) -> PromptModule {
    PromptModule {
        id: "runtime.state".to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: runtime_state_text(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::manifest::estimate_tokens_from_str;

    #[test]
    fn read_only_suffix_describes_behavior_without_claiming_authority() {
        let text = runtime_state_text(&RuntimePromptState {
            permission_mode: PermissionMode::ReadOnly,
            plan_revision: Some(3),
            plan_approved: false,
            active_contract: false,
            visual_verification_available: false,
        });

        assert!(text.contains("Plan Mode"));
        assert!(text.contains("read-only"));
        assert!(!text.contains("you are trusted to bypass"));
    }

    #[test]
    fn permission_modes_produce_bounded_non_empty_deterministic_text() {
        let modes = [
            PermissionMode::ReadOnly,
            PermissionMode::Ask,
            PermissionMode::Edits,
            PermissionMode::Auto,
            PermissionMode::AlwaysApprove,
        ];

        for mode in modes {
            let state = RuntimePromptState {
                permission_mode: mode,
                plan_revision: Some(1),
                plan_approved: true,
                active_contract: true,
                visual_verification_available: true,
            };
            let text1 = runtime_state_text(&state);
            let text2 = runtime_state_text(&state);

            assert_eq!(text1, text2);
            assert!(!text1.is_empty());
            assert!(estimate_tokens_from_str(&text1) <= 500);

            if mode == PermissionMode::AlwaysApprove {
                assert!(text1.contains("strictly apply"));
                assert!(!text1.contains("all tools are unrestricted"));
            }
        }
    }

    #[test]
    fn suffix_stays_within_token_budget() {
        let state = RuntimePromptState {
            permission_mode: PermissionMode::ReadOnly,
            plan_revision: Some(42),
            plan_approved: false,
            active_contract: true,
            visual_verification_available: false,
        };
        let text = runtime_state_text(&state);
        assert!(estimate_tokens_from_str(&text) <= 500);
    }
}
