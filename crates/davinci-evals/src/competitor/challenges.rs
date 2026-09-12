//! Claude-public-pattern challenge suite schema and corpus loading.

use crate::behavior::{BehaviorLimits, BehaviorRequirement, VerificationCommand};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeChallengeCategory {
    FrontendDistinctiveness,
    ExplorationBeforeEdit,
    AdaptiveFeatureWorkflowEfficiency,
    ReviewSpecialization,
    TriggerPrecision,
}

pub const MIN_CLAUDE_PUBLIC_CHALLENGES: usize = 150;

impl ClaudeChallengeCategory {
    pub const ALL: [Self; 5] = [
        Self::FrontendDistinctiveness,
        Self::ExplorationBeforeEdit,
        Self::AdaptiveFeatureWorkflowEfficiency,
        Self::ReviewSpecialization,
        Self::TriggerPrecision,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FrontendDistinctiveness => "frontend_distinctiveness",
            Self::ExplorationBeforeEdit => "exploration_before_edit",
            Self::AdaptiveFeatureWorkflowEfficiency => "adaptive_feature_workflow_efficiency",
            Self::ReviewSpecialization => "review_specialization",
            Self::TriggerPrecision => "trigger_precision",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaudePublicChallenge {
    pub id: String,
    pub category: ClaudeChallengeCategory,
    pub request: String,
    pub repo_fixture: String,
    #[serde(default)]
    pub expected_capability: Option<String>,
    pub expect_activation: bool,
    #[serde(default)]
    pub requirements: Vec<BehaviorRequirement>,
    #[serde(default)]
    pub limits: BehaviorLimits,
    #[serde(default)]
    pub setup_commands: Vec<String>,
    #[serde(default)]
    pub verification_commands: Vec<VerificationCommand>,
}

pub fn load_claude_public_challenges() -> Result<Vec<ClaudePublicChallenge>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root
        .join("fixtures")
        .join("competitor")
        .join("claude-public-challenges.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let challenges: Vec<ClaudePublicChallenge> = serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    validate_claude_public_challenges(&challenges, &root.join("fixtures").join("repos"))?;
    Ok(challenges)
}

fn validate_claude_public_challenges(
    challenges: &[ClaudePublicChallenge],
    repo_root: &Path,
) -> Result<(), String> {
    if challenges.len() < MIN_CLAUDE_PUBLIC_CHALLENGES {
        return Err(format!(
            "Claude public challenge suite has {} cases; at least {} are required",
            challenges.len(),
            MIN_CLAUDE_PUBLIC_CHALLENGES
        ));
    }

    let mut ids = BTreeSet::new();
    let mut category_counts = std::collections::BTreeMap::new();
    for challenge in challenges {
        if challenge.id.trim().is_empty() || !ids.insert(challenge.id.as_str()) {
            return Err(format!("duplicate or empty challenge id: {}", challenge.id));
        }
        if challenge.request.trim().is_empty() {
            return Err(format!("challenge {} has an empty request", challenge.id));
        }
        if !repo_root.join(&challenge.repo_fixture).is_dir() {
            return Err(format!(
                "challenge {} references missing fixture {}",
                challenge.id, challenge.repo_fixture
            ));
        }
        if challenge.category == ClaudeChallengeCategory::TriggerPrecision
            && challenge.expect_activation
            && challenge.expected_capability.is_none()
        {
            return Err(format!(
                "positive trigger challenge {} must name a capability",
                challenge.id
            ));
        }
        *category_counts.entry(challenge.category).or_insert(0_usize) += 1;
    }

    for category in ClaudeChallengeCategory::ALL {
        let count = category_counts.get(&category).copied().unwrap_or_default();
        if count < 20 {
            return Err(format!(
                "category {} has {} cases; at least 20 are required",
                category.as_str(),
                count
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_public_challenges_load_with_required_category_coverage() {
        let challenges = load_claude_public_challenges().expect("challenge suite must load");
        assert!(challenges.len() >= MIN_CLAUDE_PUBLIC_CHALLENGES);
        for category in ClaudeChallengeCategory::ALL {
            assert!(
                challenges
                    .iter()
                    .filter(|challenge| challenge.category == category)
                    .count()
                    >= 20,
                "category {} lacks the minimum challenge count",
                category.as_str()
            );
        }
    }

    #[test]
    fn trigger_precision_cases_have_explicit_activation_contracts() {
        let challenges = load_claude_public_challenges().expect("challenge suite must load");
        assert!(challenges
            .iter()
            .filter(|challenge| challenge.category == ClaudeChallengeCategory::TriggerPrecision)
            .all(|challenge| {
                !challenge.expect_activation || challenge.expected_capability.is_some()
            }));
        assert!(challenges.iter().any(|challenge| challenge.category
            == ClaudeChallengeCategory::TriggerPrecision
            && challenge.expect_activation));
        assert!(challenges.iter().any(|challenge| challenge.category
            == ClaudeChallengeCategory::TriggerPrecision
            && !challenge.expect_activation));
    }
}
