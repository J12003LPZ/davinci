#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def add_tests() -> None:
    path = ROOT / "crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs"
    text = path.read_text()
    if "fn worker_context_query_is_node_specific_and_deterministic()" in text:
        return
    idx = text.rfind("\n}")
    if idx < 0:
        raise SystemExit("cannot locate ecosystem context test module end")
    tests = r'''

    #[test]
    fn worker_context_query_is_node_specific_and_deterministic() {
        use crate::native_extensions::graph::Role;

        let writer = WorkerContextQuery {
            role: Some(Role::Writer),
            node_objective: "fix parser state handling".into(),
            graph_goal: "repair authentication flow".into(),
            target_hints: vec!["src/parser.rs".into(), "AuthState".into(), "src/parser.rs".into()],
            failure_hint: Some("verification failed in parser tests".into()),
        };
        let researcher = WorkerContextQuery {
            role: Some(Role::Researcher),
            node_objective: "locate token validation call sites".into(),
            graph_goal: "repair authentication flow".into(),
            target_hints: vec!["src/token.rs".into()],
            failure_hint: None,
        };

        let first = writer.render();
        assert_eq!(first, writer.render());
        assert_ne!(first, researcher.render());
        assert!(first.contains("role: writer"));
        assert!(first.contains("objective: fix parser state handling"));
        assert_eq!(first.matches("src/parser.rs").count(), 1);
        assert!(first.chars().count() <= 2_000);
    }

    #[test]
    fn context_utility_prefers_verified_relevant_value_per_token() {
        let concise = context_utility(ContextUtilityInput {
            relevance: 0.82,
            confidence: 0.95,
            applicability: 1.0,
            freshness: 1.0,
            estimated_tokens: 120,
        });
        let bloated = context_utility(ContextUtilityInput {
            relevance: 0.86,
            confidence: 0.95,
            applicability: 1.0,
            freshness: 1.0,
            estimated_tokens: 900,
        });
        let stale = context_utility(ContextUtilityInput {
            relevance: 0.95,
            confidence: 0.95,
            applicability: 1.0,
            freshness: 0.45,
            estimated_tokens: 120,
        });
        assert!(concise > bloated);
        assert!(concise > stale);
    }
'''
    path.write_text(text[:idx] + tests + text[idx:])


def apply_impl() -> None:
    path = ROOT / "crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs"
    text = path.read_text()

    marker = "pub const DEFAULT_GRAPH_SKILL_COUNT: usize = 2;\n"
    addition = r'''

pub const WORKER_CONTEXT_QUERY_MAX_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerContextQuery {
    pub role: Option<crate::native_extensions::graph::Role>,
    pub node_objective: String,
    pub graph_goal: String,
    pub target_hints: Vec<String>,
    pub failure_hint: Option<String>,
}

impl WorkerContextQuery {
    pub fn render(&self) -> String {
        fn bounded(value: &str, limit: usize) -> String {
            value.trim().chars().take(limit).collect()
        }

        let mut lines = Vec::new();
        if let Some(role) = self.role {
            lines.push(format!("role: {}", role.as_str()));
        }
        if !self.node_objective.trim().is_empty() {
            lines.push(format!(
                "objective: {}",
                bounded(&self.node_objective, 900)
            ));
        }
        let mut targets = self
            .target_hints
            .iter()
            .map(|value| bounded(value, 180))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        if !targets.is_empty() {
            lines.push(format!("targets: {}", targets.join(", ")));
        }
        if !self.graph_goal.trim().is_empty() {
            lines.push(format!("goal: {}", bounded(&self.graph_goal, 650)));
        }
        if let Some(failure) = self.failure_hint.as_deref().filter(|value| !value.trim().is_empty()) {
            lines.push(format!("failure: {}", bounded(failure, 420)));
        }
        lines
            .join("\n")
            .chars()
            .take(WORKER_CONTEXT_QUERY_MAX_CHARS)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextUtilityInput {
    pub relevance: f32,
    pub confidence: f32,
    pub applicability: f32,
    pub freshness: f32,
    pub estimated_tokens: usize,
}

pub fn context_utility(input: ContextUtilityInput) -> f32 {
    let numerator = input.relevance.clamp(0.0, 1.0)
        * input.confidence.clamp(0.0, 1.0)
        * input.applicability.clamp(0.0, 1.0)
        * input.freshness.clamp(0.0, 1.0);
    numerator / (input.estimated_tokens.max(1) as f32).sqrt()
}

fn memory_context_utility(hit: &MemoryContextHit) -> f32 {
    context_utility(ContextUtilityInput {
        relevance: hit.score,
        confidence: 0.90,
        applicability: 1.0,
        freshness: 1.0,
        estimated_tokens: hit.estimated_tokens,
    })
}

fn skill_context_utility(skill: &SkillContextCandidate) -> f32 {
    context_utility(ContextUtilityInput {
        relevance: skill.score,
        confidence: 0.95,
        applicability: 1.0,
        freshness: 1.0,
        estimated_tokens: skill.estimated_tokens,
    })
}
'''
    if "pub struct WorkerContextQuery" not in text:
        text = replace_once(text, marker, marker + addition, "worker query insertion")

    text = replace_once(
        text,
        '''            (Some(m), Some(s)) => {
                if m.score <= s.score {
                    memory_hits.pop();
                } else {
                    skill_candidates.pop();
                }
            }
''',
        '''            (Some(m), Some(s)) => {
                if memory_context_utility(m) <= skill_context_utility(s) {
                    memory_hits.pop();
                } else {
                    skill_candidates.pop();
                }
            }
''',
        "cross-source utility trim",
    )
    path.write_text(text)

    controller_path = ROOT / "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs"
    controller = controller_path.read_text()
    old = r'''                (Some(mem), Some(learn)) => {
                    let prompt = if !self.options.goal.trim().is_empty() {
                        &self.options.goal
                    } else {
                        &briefing
                    };
                    crate::native_extensions::ecosystem::select_capabilities(
                        mem,
                        learn,
                        authorized_tools,
                        crate::native_extensions::ecosystem::CapabilityRequest::new(prompt, role)
                            .with_context_token_cap(
                                crate::native_extensions::ecosystem::DEFAULT_GRAPH_CONTEXT_TOKENS,
                            )
                            .with_skills(true),
                    )
                }
'''
    new = r'''                (Some(mem), Some(learn)) => {
                    let context_query = crate::native_extensions::ecosystem::WorkerContextQuery {
                        role: Some(role),
                        node_objective: briefing.clone(),
                        graph_goal: self.options.goal.clone(),
                        target_hints: task.focus.clone().into_iter().collect(),
                        failure_hint: None,
                    }
                    .render();
                    crate::native_extensions::ecosystem::select_capabilities(
                        mem,
                        learn,
                        authorized_tools,
                        crate::native_extensions::ecosystem::CapabilityRequest::new(
                            &context_query,
                            role,
                        )
                        .with_context_token_cap(
                            crate::native_extensions::ecosystem::DEFAULT_GRAPH_CONTEXT_TOKENS,
                        )
                        .with_skills(true),
                    )
                }
'''
    controller = replace_once(controller, old, new, "node-specific capability query")
    controller_path.write_text(controller)


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tests", action="store_true")
    mode.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
