//! Capability evidence gates and completion ceilings.

use super::{CapabilityRunState, NativeBehaviorCapability};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityGateOutcome {
    AllowCompletion,
    ContinueWithReminder {
        message: String,
        reason_code: String,
    },
}

pub const MAX_CAPABILITY_COMPLETION_REMINDERS: u32 = 2;

pub const DEBUGGING_REPRODUCER_REASON_CODE: &str = "debugging.reproducer_not_verified";
pub const FRONTEND_SNAPSHOT_REASON_CODE: &str = "frontend.visual_snapshot_missing";

pub fn evaluate_completion(
    state: &CapabilityRunState,
    reminders_sent: u32,
) -> CapabilityGateOutcome {
    if let Some(reason_code) = incomplete_evidence_reason(state) {
        if reminders_sent < MAX_CAPABILITY_COMPLETION_REMINDERS {
            return CapabilityGateOutcome::ContinueWithReminder {
                message: reminder_message(reason_code).to_string(),
                reason_code: reason_code.to_string(),
            };
        }
    }

    CapabilityGateOutcome::AllowCompletion
}

pub fn incomplete_evidence_reason(state: &CapabilityRunState) -> Option<&'static str> {
    if state.active.contains(&NativeBehaviorCapability::Debugging)
        && state.debugging.as_ref().is_some_and(|debugging| {
            debugging.reproducer_failed_before_edit
                && debugging.causal_edit_seen
                && !debugging.reproducer_passed_after_edit
        })
    {
        return Some(DEBUGGING_REPRODUCER_REASON_CODE);
    }

    if state
        .active
        .contains(&NativeBehaviorCapability::FrontendDesign)
        && state.frontend.as_ref().is_some_and(|frontend| {
            frontend.frontend_edit_seen
                && frontend.visual_backend_available
                && !frontend.visual_snapshot_after_last_edit
        })
    {
        return Some(FRONTEND_SNAPSHOT_REASON_CODE);
    }

    None
}

fn reminder_message(reason_code: &str) -> &'static str {
    match reason_code {
        DEBUGGING_REPRODUCER_REASON_CODE => {
            "Before completing, rerun the original failing command unchanged and verify the result."
        }
        FRONTEND_SNAPSHOT_REASON_CODE => {
            "Before completing, capture a visual snapshot after the latest successful edit."
        }
        _ => "Before completing, gather the missing verification evidence.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::capabilities::{CapabilityDecision, DebuggingState, FrontendDesignState};

    fn state(capability: NativeBehaviorCapability) -> CapabilityRunState {
        let decision = CapabilityDecision {
            capabilities: vec![capability],
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let mut state = CapabilityRunState::default();
        state.reset_for_user_turn(&decision, false);
        state
    }

    #[test]
    fn debugging_requires_original_reproducer_after_edit() {
        let mut state = state(NativeBehaviorCapability::Debugging);
        state.debugging = Some(DebuggingState {
            reproducer_failed_before_edit: true,
            causal_edit_seen: true,
            ..DebuggingState::default()
        });

        let outcome = evaluate_completion(&state, 0);
        assert_eq!(
            outcome,
            CapabilityGateOutcome::ContinueWithReminder {
                message: "Before completing, rerun the original failing command unchanged and verify the result."
                    .into(),
                reason_code: DEBUGGING_REPRODUCER_REASON_CODE.into(),
            }
        );

        state
            .debugging
            .as_mut()
            .unwrap()
            .reproducer_passed_after_edit = true;
        assert_eq!(
            evaluate_completion(&state, 0),
            CapabilityGateOutcome::AllowCompletion
        );
    }

    #[test]
    fn frontend_requires_snapshot_only_when_backend_is_available() {
        let mut state = state(NativeBehaviorCapability::FrontendDesign);
        state.frontend = Some(FrontendDesignState {
            frontend_edit_seen: true,
            visual_backend_available: true,
            ..FrontendDesignState::default()
        });
        assert!(matches!(
            evaluate_completion(&state, 0),
            CapabilityGateOutcome::ContinueWithReminder { .. }
        ));

        state.frontend.as_mut().unwrap().visual_backend_available = false;
        assert_eq!(
            evaluate_completion(&state, 0),
            CapabilityGateOutcome::AllowCompletion
        );
    }

    #[test]
    fn review_only_completion_does_not_require_edits() {
        let state = state(NativeBehaviorCapability::CodeReview);
        assert_eq!(
            evaluate_completion(&state, 0),
            CapabilityGateOutcome::AllowCompletion
        );
    }

    #[test]
    fn debugging_without_a_causal_edit_does_not_claim_unresolved_change() {
        let mut state = state(NativeBehaviorCapability::Debugging);
        state.debugging = Some(DebuggingState {
            reproducer_failed_before_edit: true,
            ..DebuggingState::default()
        });
        assert_eq!(
            evaluate_completion(&state, 0),
            CapabilityGateOutcome::AllowCompletion
        );
    }

    #[test]
    fn max_reminders_allow_completion_but_keep_incomplete_evidence_visible() {
        let mut state = state(NativeBehaviorCapability::Debugging);
        state.debugging = Some(DebuggingState {
            reproducer_failed_before_edit: true,
            causal_edit_seen: true,
            ..DebuggingState::default()
        });
        assert_eq!(
            incomplete_evidence_reason(&state),
            Some(DEBUGGING_REPRODUCER_REASON_CODE)
        );
        assert_eq!(
            evaluate_completion(&state, MAX_CAPABILITY_COMPLETION_REMINDERS),
            CapabilityGateOutcome::AllowCompletion
        );
    }
}
