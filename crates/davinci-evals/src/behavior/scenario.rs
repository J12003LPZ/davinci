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
}
