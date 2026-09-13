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
    OriginalReproducerPassed { kind: String, command: String },
    ForbiddenDiffPattern { pattern: String },
    CapabilityActivated { capability: String },
    PromptProfile { profile: String },
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
pub struct VerificationCommand {
    pub command: String,
    pub timeout_seconds: u64,
    pub expected_exit: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorTurn {
    pub user_request: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MultiTurnBehaviorScenario {
    pub id: String,
    pub turns: Vec<BehaviorTurn>,
    #[serde(default)]
    pub verification_commands: Vec<VerificationCommand>,
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
    #[serde(default)]
    pub setup_commands: Vec<String>,
    #[serde(default)]
    pub verification_commands: Vec<VerificationCommand>,
}

pub fn load_core_200_corpus() -> Result<Vec<BehaviorScenario>, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("behavior")
        .join("core-200.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut scenarios: Vec<BehaviorScenario> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
    for scenario in &mut scenarios {
        let is_mutating = scenario
            .requirements
            .iter()
            .any(|requirement| matches!(requirement, BehaviorRequirement::FileChanged { .. }));
        if is_mutating && scenario.verification_commands.is_empty() {
            scenario.verification_commands.push(VerificationCommand {
                command: "git diff --check".into(),
                timeout_seconds: 30,
                expected_exit: 0,
            });
        }
    }
    Ok(scenarios)
}

pub fn load_debugging_hard_corpus() -> Result<Vec<BehaviorScenario>, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("debugging-hard.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {e}", path.display()))
}

pub fn load_long_horizon_corpus() -> Result<Vec<MultiTurnBehaviorScenario>, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("behavior")
        .join("long-horizon.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let scenarios: Vec<MultiTurnBehaviorScenario> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
    let mut ids = std::collections::BTreeSet::new();
    for scenario in &scenarios {
        if scenario.id.trim().is_empty() || !ids.insert(&scenario.id) {
            return Err(format!(
                "duplicate or empty long-horizon id: {}",
                scenario.id
            ));
        }
        if scenario.turns.is_empty()
            || scenario
                .turns
                .iter()
                .any(|turn| turn.user_request.trim().is_empty())
        {
            return Err(format!(
                "long-horizon scenario has invalid turns: {}",
                scenario.id
            ));
        }
    }
    if scenarios.len() != 30 {
        return Err(format!(
            "long-horizon corpus must contain exactly 30 scenarios, found {}",
            scenarios.len()
        ));
    }
    Ok(scenarios)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionOwner {
    Runtime,
    Prompt,
    CapabilityRouter,
    ToolDescription,
    ModelFamily,
    Eval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionCase {
    pub id: String,
    pub owner: RegressionOwner,
    pub issue: String,
    pub scenario: BehaviorScenario,
}

pub fn load_regression_suite(name: &str) -> Result<Vec<RegressionCase>, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("invalid regression suite name: {name}"));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("behavior")
        .join("regressions")
        .join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let cases: Vec<RegressionCase> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
    validate_regression_cases(&cases)?;
    Ok(cases)
}

pub fn load_regression_corpus() -> Result<Vec<RegressionCase>, String> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("behavior")
        .join("regressions");
    let mut paths = std::fs::read_dir(&directory)
        .map_err(|e| format!("Failed to read {}: {e}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();

    let mut cases = Vec::new();
    for path in paths {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("invalid regression fixture name: {}", path.display()))?;
        cases.extend(load_regression_suite(name)?);
    }
    validate_regression_cases(&cases)?;
    Ok(cases)
}

fn validate_regression_cases(cases: &[RegressionCase]) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("repos");
    for case in cases {
        if case.id.trim().is_empty() || !ids.insert(&case.id) {
            return Err(format!("duplicate or empty regression id: {}", case.id));
        }
        if case.issue.trim().is_empty() || case.scenario.requirements.is_empty() {
            return Err(format!(
                "regression {} is missing issue or requirements",
                case.id
            ));
        }
        if !fixture_root.join(&case.scenario.repo_fixture).is_dir() {
            return Err(format!(
                "regression {} references missing fixture {}",
                case.id, case.scenario.repo_fixture
            ));
        }
    }
    Ok(())
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
                BehaviorRequirement::OriginalReproducerPassed {
                    kind: "test".into(),
                    command: "cargo test --test reproducer".into(),
                },
                BehaviorRequirement::ForbiddenDiffPattern {
                    pattern: "unwrap_or_default".into(),
                },
                BehaviorRequirement::NoUnverifiedSuccessClaim,
            ],
            limits: BehaviorLimits {
                max_unrelated_files_changed: 0,
                max_tool_calls: Some(15),
                max_model_turns: Some(5),
            },
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
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

    #[test]
    fn debugging_hard_corpus_has_fifty_unique_scenarios_and_valid_fixtures() {
        let scenarios = load_debugging_hard_corpus().expect("debugging-hard corpus must load");
        assert_eq!(scenarios.len(), 50);
        let mut ids = std::collections::BTreeSet::new();
        let repo_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("repos");
        let mut fixture_counts = std::collections::BTreeMap::new();
        for scenario in scenarios {
            *fixture_counts
                .entry(scenario.repo_fixture.clone())
                .or_insert(0_usize) += 1;
            assert!(
                ids.insert(scenario.id.clone()),
                "duplicate id: {}",
                scenario.id
            );
            assert!(
                scenario.requirements.iter().any(|requirement| matches!(
                    requirement,
                    BehaviorRequirement::OriginalReproducerPassed { .. }
                )),
                "{} lacks original reproducer requirement",
                scenario.id
            );
            assert!(
                scenario.requirements.iter().any(|requirement| matches!(
                    requirement,
                    BehaviorRequirement::ForbiddenDiffPattern { .. }
                )),
                "{} lacks forbidden diff requirement",
                scenario.id
            );
            let fixture_path = repo_dir.join(&scenario.repo_fixture);
            assert!(fixture_path.is_dir());
            assert!(
                fixture_path.join("Cargo.toml").is_file(),
                "{} is not a runnable Rust fixture",
                scenario.repo_fixture
            );
            assert!(
                std::fs::read_dir(fixture_path.join("src"))
                    .expect("fixture src directory must be readable")
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().extension().is_some_and(|ext| ext == "rs")),
                "{} has no Rust source file",
                scenario.repo_fixture
            );
            assert!(
                scenario.requirements.iter().any(|requirement| matches!(
                    requirement,
                    BehaviorRequirement::OriginalReproducerPassed { kind, command }
                        if kind == "test" && command == "cargo test --quiet"
                )),
                "{} lacks an exact runnable reproducer contract",
                scenario.id
            );
        }
        assert_eq!(fixture_counts.len(), 10);
        assert!(fixture_counts.values().all(|count| *count == 5));
    }

    #[test]
    fn mutating_core_scenarios_have_independent_verification() {
        let scenarios = load_core_200_corpus().expect("core-200 corpus must load");
        let mut mutating = 0;
        for scenario in scenarios {
            if scenario
                .requirements
                .iter()
                .any(|requirement| matches!(requirement, BehaviorRequirement::FileChanged { .. }))
            {
                mutating += 1;
                assert_eq!(scenario.verification_commands.len(), 1);
                assert_eq!(
                    scenario.verification_commands[0].command,
                    "git diff --check"
                );
            }
        }
        assert_eq!(mutating, 90);
    }

    #[test]
    fn regression_suite_loads_named_cases_and_rejects_path_traversal() {
        let cases = load_regression_suite("dormant-capability-profile").unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].owner, RegressionOwner::CapabilityRouter);
        assert!(load_regression_suite("../core-200").is_err());
        let corpus = load_regression_corpus().unwrap();
        assert!(corpus.len() >= cases.len());
        assert!(corpus.iter().any(|case| case.id == cases[0].id));
    }

    #[test]
    fn long_horizon_corpus_has_thirty_valid_scenarios() {
        let scenarios = load_long_horizon_corpus().expect("long-horizon corpus must load");
        assert_eq!(scenarios.len(), 30);
        assert!(scenarios.iter().all(|scenario| scenario.turns.len() >= 5));
    }

    #[test]
    fn gpt6_astra_regression_suite_is_valid() {
        let cases = load_regression_suite("gpt6-astra").expect("gpt6-astra suite");
        assert_eq!(cases.len(), 4);
        assert!(cases
            .iter()
            .all(|case| case.owner == RegressionOwner::ModelFamily));
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/repos/astra-policy-rust/Cargo.lock");
        assert!(fixture.is_file(), "Astra fixture must pre-seed Cargo.lock");
    }
}
