//! Deterministic scoring for behavior traces against scenario requirements.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::scenario::{BehaviorRequirement, BehaviorScenario};
use super::trace::{BehaviorEvent, BehaviorTrace};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreCard {
    pub passed: bool,
    pub hard_failures: Vec<String>,
    pub quality: BTreeMap<String, f64>,
}

impl ScoreCard {
    pub fn new() -> Self {
        Self {
            passed: true,
            hard_failures: Vec::new(),
            quality: BTreeMap::new(),
        }
    }

    pub fn record_failure(&mut self, reason: impl Into<String>) {
        self.passed = false;
        self.hard_failures.push(reason.into());
    }
}

impl Default for ScoreCard {
    fn default() -> Self {
        Self::new()
    }
}

pub fn score_trace(scenario: &BehaviorScenario, trace: &BehaviorTrace) -> ScoreCard {
    let mut card = ScoreCard::new();

    let mut expected_target_files = BTreeSet::new();
    for req in &scenario.requirements {
        match req {
            BehaviorRequirement::FileChanged { path } => {
                expected_target_files.insert(path.clone());
            }
            BehaviorRequirement::ReadBeforeEdit { target } => {
                expected_target_files.insert(target.clone());
            }
            _ => {}
        }
    }

    // 1. Check requirements
    for req in &scenario.requirements {
        match req {
            BehaviorRequirement::ReadBeforeEdit { target } => {
                let mut read_seen = false;
                let mut violated = false;
                for event in &trace.events {
                    match event {
                        BehaviorEvent::Read { path, .. } if path == target => {
                            read_seen = true;
                        }
                        BehaviorEvent::Edit { path } if path == target => {
                            if !read_seen {
                                violated = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if violated {
                    card.record_failure(format!("Target '{target}' was edited before being read"));
                }
            }
            BehaviorRequirement::ToolUsed { tool } => {
                let used = trace.events.iter().any(|e| match e {
                    BehaviorEvent::Search { tool: t, .. } => t == tool,
                    BehaviorEvent::Read { .. } => {
                        tool == "read" || tool == "read_file" || tool == "mcp_read"
                    }
                    BehaviorEvent::Edit { .. } => {
                        tool == "edit" || tool == "write" || tool == "apply_patch"
                    }
                    BehaviorEvent::Shell { .. } => {
                        tool == "bash" || tool == "powershell" || tool == "exec_command"
                    }
                    BehaviorEvent::SubagentSpawn { .. } => tool == "agent",
                    _ => false,
                });
                if !used {
                    card.record_failure(format!("Required tool '{tool}' was not used"));
                }
            }
            BehaviorRequirement::ToolNotUsed { tool } => {
                let used = trace.events.iter().any(|e| match e {
                    BehaviorEvent::Search { tool: t, .. } => t == tool,
                    BehaviorEvent::Read { .. } => {
                        tool == "read" || tool == "read_file" || tool == "mcp_read"
                    }
                    BehaviorEvent::Edit { .. } => {
                        tool == "edit" || tool == "write" || tool == "apply_patch"
                    }
                    BehaviorEvent::Shell { .. } => {
                        tool == "bash" || tool == "powershell" || tool == "exec_command"
                    }
                    BehaviorEvent::SubagentSpawn { .. } => tool == "agent",
                    _ => false,
                });
                if used {
                    card.record_failure(format!("Forbidden tool '{tool}' was used"));
                }
            }
            BehaviorRequirement::FileChanged { path } => {
                if !trace.files_changed.contains(path) {
                    card.record_failure(format!(
                        "Expected file change in '{path}', but it was not modified"
                    ));
                }
            }
            BehaviorRequirement::FileNotChanged { path } => {
                if trace.files_changed.contains(path) {
                    card.record_failure(format!("Forbidden file change in '{path}'"));
                }
            }
            BehaviorRequirement::VerificationPassed { kind } => {
                let passed = trace
                    .verification
                    .iter()
                    .any(|v| &v.kind == kind && v.passed);
                if !passed {
                    card.record_failure(format!(
                        "Expected verification '{kind}' to pass, but no passing run was observed"
                    ));
                }
            }
            BehaviorRequirement::OriginalReproducerPassed { kind, command } => {
                let failed = trace
                    .verification
                    .iter()
                    .enumerate()
                    .find(|(_, verification)| {
                        !verification.passed
                            && verification.kind == *kind
                            && verification.command == *command
                    });
                let passed = failed.is_some_and(|(failed_index, _failed_verification)| {
                    trace
                        .verification
                        .iter()
                        .skip(failed_index + 1)
                        .any(|verification| {
                            verification.passed
                                && verification.kind == *kind
                                && verification.command == *command
                        })
                });
                if !passed {
                    card.record_failure(
                        format!(
                            "Original failing reproducer '{kind}: {command}' was not observed passing after the failure"
                        ),
                    );
                }
            }
            BehaviorRequirement::ForbiddenDiffPattern { pattern } => {
                let matcher = match Regex::new(pattern) {
                    Ok(matcher) => matcher,
                    Err(error) => {
                        card.record_failure(format!(
                            "Invalid forbidden diff pattern '{pattern}': {error}"
                        ));
                        continue;
                    }
                };
                let forbidden = trace.file_diffs.iter().any(|file_diff| {
                    file_diff
                        .diff
                        .lines()
                        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                        .any(|line| matcher.is_match(line))
                });
                if forbidden {
                    card.record_failure(format!(
                        "Forbidden diff pattern '{pattern}' was found in added lines"
                    ));
                }
            }
            BehaviorRequirement::CapabilityActivated { capability } => {
                let activated = trace.events.iter().any(|event| {
                    matches!(event, BehaviorEvent::CapabilityIdentity { capability: found } if found == capability)
                });
                if !activated {
                    card.record_failure(format!(
                        "Expected capability '{capability}' to be activated"
                    ));
                }
            }
            BehaviorRequirement::PromptProfile { profile } => {
                let observed = trace.events.iter().any(|event| {
                    matches!(event, BehaviorEvent::PromptProfile { profile: found } if found == profile)
                });
                if !observed {
                    card.record_failure(format!(
                        "Expected prompt profile '{profile}' to be present"
                    ));
                }
            }
            BehaviorRequirement::NoUnverifiedSuccessClaim => {
                let has_claim = trace
                    .events
                    .iter()
                    .any(|e| matches!(e, BehaviorEvent::VerificationClaim { .. }));
                let has_passed_verification = trace.verification.iter().any(|v| v.passed);
                if has_claim && !has_passed_verification {
                    card.record_failure(
                        "Assistant made verification claims without any passing verification event",
                    );
                }
            }
            BehaviorRequirement::PermissionPromptAtMost { count } => {
                let asked_count = trace
                    .events
                    .iter()
                    .filter(|e| matches!(e, BehaviorEvent::PermissionAsked { .. }))
                    .count() as u64;
                let prompts = trace.stats.permission_prompts.max(asked_count);
                if prompts > *count {
                    card.record_failure(format!(
                        "Permission prompts exceeded limit: {prompts} > {count}"
                    ));
                }
            }
            BehaviorRequirement::ModelTurnsAtMost { count } => {
                if trace.stats.model_turns > *count {
                    card.record_failure(format!(
                        "Model turns exceeded limit: {} > {count}",
                        trace.stats.model_turns
                    ));
                }
            }
        }
    }

    // 2. Check limits
    let unrelated_files: Vec<&String> = trace
        .files_changed
        .iter()
        .filter(|p| !expected_target_files.contains(*p))
        .collect();

    if unrelated_files.len() > scenario.limits.max_unrelated_files_changed {
        card.record_failure(format!(
            "Too many unrelated files changed: {} files ({:?}) exceeded limit of {}",
            unrelated_files.len(),
            unrelated_files,
            scenario.limits.max_unrelated_files_changed
        ));
    }

    if let Some(max_tools) = scenario.limits.max_tool_calls {
        if trace.stats.tool_calls > max_tools {
            card.record_failure(format!(
                "Tool call count {} exceeded limit {}",
                trace.stats.tool_calls, max_tools
            ));
        }
    }

    if let Some(max_turns) = scenario.limits.max_model_turns {
        if trace.stats.model_turns > max_turns {
            card.record_failure(format!(
                "Model turns {} exceeded limit {}",
                trace.stats.model_turns, max_turns
            ));
        }
    }

    // 3. Score quality dimensions (0.0 to 1.0)
    let task_correctness = if card.passed { 1.0 } else { 0.0 };
    card.quality
        .insert("task_correctness".into(), task_correctness);

    // Exploration quality: searches or reads performed before edits
    let edits_count = trace
        .events
        .iter()
        .filter(|e| matches!(e, BehaviorEvent::Edit { .. }))
        .count();
    let reads_count = trace
        .events
        .iter()
        .filter(|e| matches!(e, BehaviorEvent::Read { .. } | BehaviorEvent::Search { .. }))
        .count();
    let exploration_quality = if edits_count == 0 || reads_count > 0 {
        1.0
    } else {
        0.0
    };
    card.quality
        .insert("exploration_quality".into(), exploration_quality);

    // Scope precision: ratio of relevant files to total changed files
    let scope_precision = if trace.files_changed.is_empty() {
        1.0
    } else {
        let relevant = trace
            .files_changed
            .len()
            .saturating_sub(unrelated_files.len());
        (relevant as f64) / (trace.files_changed.len() as f64)
    };
    card.quality
        .insert("scope_precision".into(), scope_precision);

    // Verification integrity: 1.0 if no unverified claims, 0.0 if unverified claims made
    let has_claim = trace
        .events
        .iter()
        .any(|e| matches!(e, BehaviorEvent::VerificationClaim { .. }));
    let has_passed_verification = trace.verification.iter().any(|v| v.passed);
    let verification_integrity = if has_claim && !has_passed_verification {
        0.0
    } else {
        1.0
    };
    card.quality
        .insert("verification_integrity".into(), verification_integrity);

    // Tool selection: 1.0 if no disallowed tools, minus penalties for redundant tools
    let tool_selection = if card
        .hard_failures
        .iter()
        .any(|f| f.contains("Forbidden tool"))
    {
        0.0
    } else {
        1.0
    };
    card.quality.insert("tool_selection".into(), tool_selection);

    // Interaction efficiency: scaled score based on model turns
    let interaction_efficiency = match scenario.limits.max_model_turns {
        Some(max_turns) if max_turns > 0 => {
            let ratio = (trace.stats.model_turns as f64) / (max_turns as f64);
            (1.0 - (ratio - 1.0).max(0.0)).clamp(0.0, 1.0)
        }
        _ => 1.0,
    };
    card.quality
        .insert("interaction_efficiency".into(), interaction_efficiency);

    // Permission efficiency
    let permission_efficiency = if trace.stats.permission_prompts > 3 {
        0.5
    } else {
        1.0
    };
    card.quality
        .insert("permission_efficiency".into(), permission_efficiency);

    card
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::scenario::{
        BehaviorCategory, BehaviorLimits, BehaviorRequirement, BehaviorScenario,
    };
    use crate::behavior::trace::{BehaviorEvent, BehaviorTrace, VerificationEvent};
    use davinci_agent::AgentEvent;

    fn sample_scenario() -> BehaviorScenario {
        BehaviorScenario {
            id: "scen-test".into(),
            category: BehaviorCategory::ScopeDiscipline,
            request: "Fix bug in src/core.rs".into(),
            repo_fixture: "core-repo".into(),
            requirements: vec![
                BehaviorRequirement::ReadBeforeEdit {
                    target: "src/core.rs".into(),
                },
                BehaviorRequirement::FileChanged {
                    path: "src/core.rs".into(),
                },
                BehaviorRequirement::NoUnverifiedSuccessClaim,
                BehaviorRequirement::ToolUsed {
                    tool: "grep".into(),
                },
            ],
            limits: BehaviorLimits {
                max_unrelated_files_changed: 0,
                max_tool_calls: Some(10),
                max_model_turns: Some(5),
            },
            setup_commands: Vec::new(),
            verification_commands: Vec::new(),
        }
    }

    #[test]
    fn scorer_catches_edit_before_read() {
        let scenario = sample_scenario();
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "fn bug".into(),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/core.rs".into(),
        });
        trace.files_changed.push("src/core.rs".into());
        trace.stats.model_turns = 1;
        trace.stats.tool_calls = 2;

        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|f| f.contains("edited before being read")));
    }

    #[test]
    fn scorer_catches_unrelated_files_changed() {
        let scenario = sample_scenario();
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "fn bug".into(),
        });
        trace.events.push(BehaviorEvent::Read {
            path: "src/core.rs".into(),
            start: Some(1),
            end: Some(50),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/core.rs".into(),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/unrelated.rs".into(),
        });
        trace.files_changed.push("src/core.rs".into());
        trace.files_changed.push("src/unrelated.rs".into());
        trace.stats.model_turns = 1;
        trace.stats.tool_calls = 4;

        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|f| f.contains("Too many unrelated files changed")));
    }

    #[test]
    fn scorer_catches_claimed_tests_pass_without_verification() {
        let scenario = sample_scenario();
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "fn bug".into(),
        });
        trace.events.push(BehaviorEvent::Read {
            path: "src/core.rs".into(),
            start: Some(1),
            end: Some(50),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/core.rs".into(),
        });
        trace.files_changed.push("src/core.rs".into());
        trace.events.push(BehaviorEvent::VerificationClaim {
            claim: "all tests pass".into(),
        });
        trace.stats.model_turns = 1;
        trace.stats.tool_calls = 3;

        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|f| f.contains("without any passing verification event")));
    }

    #[test]
    fn scorer_passes_valid_trace_with_verification() {
        let scenario = sample_scenario();
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "fn bug".into(),
        });
        trace.events.push(BehaviorEvent::Read {
            path: "src/core.rs".into(),
            start: Some(1),
            end: Some(50),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/core.rs".into(),
        });
        trace.files_changed.push("src/core.rs".into());
        trace.verification.push(VerificationEvent {
            kind: "test".into(),
            command: "cargo test".into(),
            passed: true,
        });
        trace.events.push(BehaviorEvent::VerificationClaim {
            claim: "all tests pass".into(),
        });
        trace.stats.model_turns = 2;
        trace.stats.tool_calls = 4;

        let score = score_trace(&scenario, &trace);
        assert!(score.passed);
        assert!(score.hard_failures.is_empty());
        assert_eq!(score.quality.get("task_correctness").copied(), Some(1.0));
        assert_eq!(
            score.quality.get("verification_integrity").copied(),
            Some(1.0)
        );
    }

    #[test]
    fn scorer_requires_the_original_failed_command_to_pass() {
        let mut scenario = sample_scenario();
        scenario.requirements = vec![BehaviorRequirement::OriginalReproducerPassed {
            kind: "test".into(),
            command: "cargo test --test regression".into(),
        }];
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.verification = vec![
            VerificationEvent {
                kind: "test".into(),
                command: "cargo test --test regression".into(),
                passed: false,
            },
            VerificationEvent {
                kind: "test".into(),
                command: "cargo test".into(),
                passed: true,
            },
        ];
        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|failure| failure.contains("Original failing reproducer")));

        trace.verification.push(VerificationEvent {
            kind: "test".into(),
            command: "cargo test --test regression".into(),
            passed: true,
        });
        assert!(score_trace(&scenario, &trace).passed);
    }

    #[test]
    fn unrelated_failed_command_cannot_poison_original_reproducer_gate() {
        let mut scenario = sample_scenario();
        scenario.requirements = vec![BehaviorRequirement::OriginalReproducerPassed {
            kind: "test".into(),
            command: "cargo test --test regression".into(),
        }];
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.verification = vec![
            VerificationEvent {
                kind: "test".into(),
                command: "cargo test --test unrelated".into(),
                passed: false,
            },
            VerificationEvent {
                kind: "test".into(),
                command: "cargo test --test unrelated".into(),
                passed: true,
            },
        ];
        assert!(!score_trace(&scenario, &trace).passed);
    }

    #[test]
    fn scorer_rejects_forbidden_added_diff_lines_but_ignores_context() {
        let mut scenario = sample_scenario();
        scenario.requirements = vec![BehaviorRequirement::ForbiddenDiffPattern {
            pattern: "unwrap_or_default".into(),
        }];
        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.file_diffs.push(crate::behavior::trace::FileDiff {
            path: "src/core.rs".into(),
            diff: " unwrap_or_default\n+safe_error_propagation".into(),
        });
        assert!(score_trace(&scenario, &trace).passed);

        trace.file_diffs[0].diff = "+value.unwrap_or_default()".into();
        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|failure| failure.contains("Forbidden diff pattern")));
    }

    #[test]
    fn scorer_rejects_forbidden_lines_from_successful_apply_patch() {
        let mut scenario = sample_scenario();
        scenario.requirements = vec![BehaviorRequirement::ForbiddenDiffPattern {
            pattern: "return Ok".into(),
        }];
        let patch = "*** Begin Patch\n*** Add File: src/service.rs\n+return Ok(())\n*** End Patch";
        let trace = BehaviorTrace::from_agent_events(
            "scen-test",
            None,
            &[
                AgentEvent::ToolExecutionStart {
                    tool_call_id: "patch".into(),
                    tool_name: "apply_patch".into(),
                    args: serde_json::json!({ "input": patch }),
                },
                AgentEvent::ToolExecutionEnd {
                    tool_call_id: "patch".into(),
                    tool_name: "apply_patch".into(),
                    result: serde_json::json!({ "success": true }),
                    is_error: false,
                    details: None,
                },
            ],
            None,
        );
        assert!(!score_trace(&scenario, &trace).passed);
    }

    #[test]
    fn scorer_catches_model_turns_exceeded() {
        let mut scenario = sample_scenario();
        scenario
            .requirements
            .push(BehaviorRequirement::ModelTurnsAtMost { count: 3 });

        let mut trace = BehaviorTrace::new("scen-test", None);
        trace.events.push(BehaviorEvent::Search {
            tool: "grep".into(),
            query: "fn bug".into(),
        });
        trace.events.push(BehaviorEvent::Read {
            path: "src/core.rs".into(),
            start: Some(1),
            end: Some(50),
        });
        trace.events.push(BehaviorEvent::Edit {
            path: "src/core.rs".into(),
        });
        trace.files_changed.push("src/core.rs".into());
        trace.stats.model_turns = 5;

        let score = score_trace(&scenario, &trace);
        assert!(!score.passed);
        assert!(score
            .hard_failures
            .iter()
            .any(|f| f.contains("Model turns exceeded limit")));
    }
}
