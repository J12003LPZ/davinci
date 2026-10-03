use super::super::ProcessOperationBinding;
use serde::{Deserialize, Serialize};

/// Durable identity attached to filesystem transaction records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemOperationLink {
    pub operation_id: String,
    pub attempt_id: String,
    pub owner_generation: u64,
}

impl FilesystemOperationLink {
    pub fn from_binding(binding: &ProcessOperationBinding) -> Self {
        Self {
            operation_id: binding.operation_id.to_string(),
            attempt_id: binding.attempt_id.to_string(),
            owner_generation: binding.owner_generation,
        }
    }
}

/// What a recovery observer can prove about a path after a crash or interrupted
/// multi-path mutation. The observation is deliberately separate from the
/// transaction receipt: seeing desired bytes does not fabricate command history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemEffectObservation {
    NotStarted,
    Preimage,
    AppliedPostimage,
    Partial,
    ThirdParty,
    Unknown,
}

pub fn classify_file_observation(
    before_hash: Option<&str>,
    desired_hash: Option<&str>,
    current_hash: Option<&str>,
    mutation_recorded: bool,
) -> FilesystemEffectObservation {
    if current_hash == before_hash {
        return if mutation_recorded {
            FilesystemEffectObservation::Preimage
        } else {
            FilesystemEffectObservation::NotStarted
        };
    }
    if current_hash == desired_hash {
        return FilesystemEffectObservation::AppliedPostimage;
    }
    if mutation_recorded {
        FilesystemEffectObservation::Partial
    } else {
        FilesystemEffectObservation::ThirdParty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_keeps_preimage_and_postimage_distinct() {
        assert_eq!(
            classify_file_observation(Some("before"), Some("after"), Some("before"), false),
            FilesystemEffectObservation::NotStarted
        );
        assert_eq!(
            classify_file_observation(Some("before"), Some("after"), Some("before"), true),
            FilesystemEffectObservation::Preimage
        );
        assert_eq!(
            classify_file_observation(Some("before"), Some("after"), Some("after"), false),
            FilesystemEffectObservation::AppliedPostimage
        );
        assert_eq!(
            classify_file_observation(Some("before"), Some("after"), Some("other"), true),
            FilesystemEffectObservation::Partial
        );
        assert_eq!(
            classify_file_observation(Some("before"), Some("after"), Some("other"), false),
            FilesystemEffectObservation::ThirdParty
        );
    }
}
