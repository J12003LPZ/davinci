//! Behavior evaluation scenario schema and requirements.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorCategory {
    Exploration,
    ScopeDiscipline,
    VerificationIntegrity,
    ToolSelection,
    Collaboration,
    Planning,
    SecurityBoundary,
    FrontendCapability,
    AntiOverengineering,
}

impl BehaviorCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exploration => "exploration",
            Self::ScopeDiscipline => "scope_discipline",
            Self::VerificationIntegrity => "verification_integrity",
            Self::ToolSelection => "tool_selection",
            Self::Collaboration => "collaboration",
            Self::Planning => "planning",
            Self::SecurityBoundary => "security_boundary",
            Self::FrontendCapability => "frontend_capability",
            Self::AntiOverengineering => "anti_overengineering",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BehaviorRequirement {
    ReadBeforeEdit { target: String },
    ToolUsed { tool: String },
    ToolNotUsed { tool: String },
    FileChanged { path: String },
    FileNotChanged { path: String },
    VerificationPassed { kind: String },
    NoUnverifiedSuccessClaim,
    PermissionPromptAtMost { count: u64 },
    ModelTurnsAtMost { count: u64 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorLimits {
    #[serde(default)]
    pub max_unrelated_files_changed: usize,
    #[serde(default)]
    pub max_tool_calls: Option<u64>,
    #[serde(default)]
    pub max_model_turns: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorScenario {
    pub id: String,
    pub category: BehaviorCategory,
    pub request: String,
    pub repo_fixture: String,
    #[serde(default)]
    pub requirements: Vec<BehaviorRequirement>,
    #[serde(default)]
    pub limits: BehaviorLimits,
}

pub fn load_core_200_corpus() -> Result<Vec<BehaviorScenario>, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("behavior")
        .join("core-200.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_serde_roundtrip() {
        let scenario = BehaviorScenario {
            id: "explore-01".into(),
            category: BehaviorCategory::Exploration,
            request: "Fix bug in parser".into(),
            repo_fixture: "sample-repo".into(),
            requirements: vec![
                BehaviorRequirement::ReadBeforeEdit {
                    target: "src/parser.rs".into(),
                },
                BehaviorRequirement::ToolUsed {
                    tool: "grep".into(),
                },
                BehaviorRequirement::NoUnverifiedSuccessClaim,
            ],
            limits: BehaviorLimits {
                max_unrelated_files_changed: 0,
                max_tool_calls: Some(15),
                max_model_turns: Some(5),
            },
        };

        let json = serde_json::to_string(&scenario).unwrap();
        let decoded: BehaviorScenario = serde_json::from_str(&json).unwrap();
        assert_eq!(scenario, decoded);
    }

    #[test]
    fn test_core_200_corpus_validation() {
        let scenarios = load_core_200_corpus().expect("core-200 corpus must load");
        assert_eq!(scenarios.len(), 200, "Must contain exactly 200 scenarios");

        let mut ids = std::collections::BTreeSet::new();
        let mut category_counts = std::collections::BTreeMap::new();
        let repo_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("repos");

        for scen in &scenarios {
            assert!(ids.insert(&scen.id), "Duplicate scenario id: {}", scen.id);
            *category_counts.entry(scen.category).or_insert(0) += 1;
            assert!(
                !scen.requirements.is_empty(),
                "Scenario {} has no requirements",
                scen.id
            );
            let repo_path = repo_dir.join(&scen.repo_fixture);
            assert!(
                repo_path.exists(),
                "Missing repo fixture for scenario {}: {}",
                scen.id,
                repo_path.display()
            );
        }

        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::Exploration)
                .unwrap_or(&0),
            25
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::ScopeDiscipline)
                .unwrap_or(&0),
            25
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::VerificationIntegrity)
                .unwrap_or(&0),
            25
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::ToolSelection)
                .unwrap_or(&0),
            25
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::Collaboration)
                .unwrap_or(&0),
            20
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::Planning)
                .unwrap_or(&0),
            20
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::SecurityBoundary)
                .unwrap_or(&0),
            20
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::FrontendCapability)
                .unwrap_or(&0),
            20
        );
        assert_eq!(
            *category_counts
                .get(&BehaviorCategory::AntiOverengineering)
                .unwrap_or(&0),
            20
        );
    }
}
