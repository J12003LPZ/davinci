//! Crash-safe reconciliation for script child effects.
use crate::runtime::operations::{EffectStatus, OperationJournal, OperationState};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptEffectResolution {
    NoEffect {
        operation_ref: String,
    },
    Succeeded {
        operation_ref: String,
    },
    RecoveryRequired {
        operation_ref: String,
        reason: String,
    },
}

impl ScriptEffectResolution {
    pub fn may_retry(&self) -> bool {
        matches!(self, Self::NoEffect { .. })
    }

    pub fn requires_postcondition(&self) -> bool {
        matches!(self, Self::RecoveryRequired { .. })
    }
}

/// Reduce one recorded attempt to a resolution. Ambiguity is never success.
pub fn resolve_script_effect(
    state: OperationState,
    effect: EffectStatus,
) -> ScriptEffectResolution {
    let operation_ref = String::new();
    match (state, effect) {
        (OperationState::Succeeded, _) => ScriptEffectResolution::Succeeded { operation_ref },
        (_, EffectStatus::KnownNoEffect) | (_, EffectStatus::Compensated) => {
            ScriptEffectResolution::NoEffect { operation_ref }
        }
        (
            OperationState::Failed
            | OperationState::Cancelled
            | OperationState::Blocked
            | OperationState::Superseded,
            EffectStatus::NotStarted,
        ) => ScriptEffectResolution::NoEffect { operation_ref },
        (state, effect) => ScriptEffectResolution::RecoveryRequired {
            operation_ref,
            reason: format!(
                "operation is {state:?} with effect {effect:?}; the outcome is not proven"
            ),
        },
    }
}

/// Resolve one script child against the durable journal. A mutating child with
/// no durable record at all is ambiguous, not a no-op.
pub fn reconcile_script_child_effect(
    journal: &OperationJournal,
    operation_ref: &str,
) -> ScriptEffectResolution {
    let Ok(snapshot) = journal.snapshot() else {
        return ScriptEffectResolution::RecoveryRequired {
            operation_ref: operation_ref.into(),
            reason: "journal snapshot unavailable".into(),
        };
    };
    let Some(attempt) = snapshot
        .attempts
        .iter()
        .find(|attempt| attempt.operation_id().to_string() == operation_ref)
    else {
        return ScriptEffectResolution::RecoveryRequired {
            operation_ref: operation_ref.into(),
            reason: "no durable journal record for the child effect".into(),
        };
    };
    match resolve_script_effect(attempt.state(), attempt.effect_status()) {
        ScriptEffectResolution::NoEffect { .. } => ScriptEffectResolution::NoEffect {
            operation_ref: operation_ref.into(),
        },
        ScriptEffectResolution::Succeeded { .. } => ScriptEffectResolution::Succeeded {
            operation_ref: operation_ref.into(),
        },
        ScriptEffectResolution::RecoveryRequired { reason, .. } => {
            ScriptEffectResolution::RecoveryRequired {
                operation_ref: operation_ref.into(),
                reason,
            }
        }
    }
}
