#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates/davinci-coding-agent"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    path.write_text(text.replace(old, new, 1))


def append_once(path: Path, marker: str, addition: str) -> None:
    text = path.read_text()
    if marker in text:
        return
    path.write_text(text.rstrip() + "\n\n" + addition.strip() + "\n")


def add_tests() -> None:
    context = CRATE / "src/native_extensions/ecosystem/context.rs"
    append_once(
        context,
        "skill_query_is_not_diluted_by_worker_briefing",
        r'''
#[cfg(test)]
mod skill_query_separation_regressions {
    use super::*;

    #[test]
    fn skill_query_is_not_diluted_by_worker_briefing() {
        let query = WorkerContextQuery {
            role: Some(crate::native_extensions::graph::Role::Writer),
            node_objective: "# Implement\n\n## Goal\nshared workflow rust debugging\n\n## Mode\nThis was classified trivial: implement the goal directly, smallest reasonable change.\n\n## Hard rules\n- You are the only process allowed to modify files.\n- Never run git commit, git push, or any git state change.\n- Run the plan's tests yourself before submitting.\n- If the plan cannot work as written, report the reason."
                .into(),
            graph_goal: "shared workflow rust debugging".into(),
            target_hints: vec![],
            failure_hint: None,
        };

        let skill_query = query.render_skill_query();
        assert_eq!(skill_query, "shared workflow rust debugging");
        assert!(!skill_query.contains("Hard rules"));
        assert!(!skill_query.contains("git commit"));
    }
}
''',
    )


def apply_impl() -> None:
    context = CRATE / "src/native_extensions/ecosystem/context.rs"

    replace_once(
        context,
        '''    }\n}\n\n#[derive(Debug, Clone, Copy, PartialEq)]\npub struct ContextUtilityInput {\n''',
        '''    }\n\n    /// Render a compact query for reusable skill retrieval. Worker briefings contain\n    /// policy text and execution instructions that are useful to memory retrieval but\n    /// can dilute lexical skill relevance. Prefer the user goal and task-local signals;\n    /// fall back to the node objective only when no stronger signal exists.\n    pub fn render_skill_query(&self) -> String {\n        fn bounded(value: &str, limit: usize) -> String {\n            value.trim().chars().take(limit).collect()\n        }\n\n        let mut lines = Vec::new();\n        if !self.graph_goal.trim().is_empty() {\n            lines.push(bounded(&self.graph_goal, 900));\n        }\n\n        let mut targets = self\n            .target_hints\n            .iter()\n            .map(|value| bounded(value, 180))\n            .filter(|value| !value.is_empty())\n            .collect::<Vec<_>>();\n        targets.sort();\n        targets.dedup();\n        if !targets.is_empty() {\n            lines.push(targets.join(" "));\n        }\n\n        if let Some(failure) = self\n            .failure_hint\n            .as_deref()\n            .filter(|value| !value.trim().is_empty())\n        {\n            lines.push(bounded(failure, 420));\n        }\n\n        if lines.is_empty() && !self.node_objective.trim().is_empty() {\n            lines.push(bounded(&self.node_objective, 900));\n        }\n\n        lines\n            .join("\\n")\n            .chars()\n            .take(WORKER_CONTEXT_QUERY_MAX_CHARS)\n            .collect()\n    }\n}\n\n#[derive(Debug, Clone, Copy, PartialEq)]\npub struct ContextUtilityInput {\n''',
        "add compact skill query renderer",
    )

    replace_once(
        context,
        '''pub struct ContextPacketRequest<'a> {\n    pub prompt: &'a str,\n    pub role: Option<crate::native_extensions::graph::Role>,\n''',
        '''pub struct ContextPacketRequest<'a> {\n    pub prompt: &'a str,\n    pub skill_prompt: Option<&'a str>,\n    pub role: Option<crate::native_extensions::graph::Role>,\n''',
        "add context skill prompt field",
    )
    replace_once(
        context,
        '''        Self {\n            prompt,\n            role: None,\n''',
        '''        Self {\n            prompt,\n            skill_prompt: None,\n            role: None,\n''',
        "default context skill prompt",
    )
    replace_once(
        context,
        '''    pub fn with_role(mut self, role: crate::native_extensions::graph::Role) -> Self {\n        self.role = Some(role);\n        self\n    }\n\n    pub fn with_token_cap''',
        '''    pub fn with_role(mut self, role: crate::native_extensions::graph::Role) -> Self {\n        self.role = Some(role);\n        self\n    }\n\n    pub fn with_skill_prompt(mut self, prompt: &'a str) -> Self {\n        self.skill_prompt = Some(prompt);\n        self\n    }\n\n    pub fn with_token_cap''',
        "add context skill prompt builder",
    )
    replace_once(
        context,
        '''    let mut skill_candidates = if request.include_skills {\n        let role = request\n            .role\n            .unwrap_or(crate::native_extensions::graph::Role::Writer);\n        learning.graph_skill_candidates(request.prompt, role, DEFAULT_GRAPH_SKILL_COUNT, skill_cap)\n''',
        '''    let mut skill_candidates = if request.include_skills {\n        let role = request\n            .role\n            .unwrap_or(crate::native_extensions::graph::Role::Writer);\n        let skill_prompt = request.skill_prompt.unwrap_or(request.prompt);\n        learning.graph_skill_candidates(skill_prompt, role, DEFAULT_GRAPH_SKILL_COUNT, skill_cap)\n''',
        "route skill retrieval through compact prompt",
    )

    capability = CRATE / "src/native_extensions/ecosystem/capability.rs"
    replace_once(
        capability,
        '''pub struct CapabilityRequest<'a> {\n    pub prompt: &'a str,\n    pub role: Role,\n''',
        '''pub struct CapabilityRequest<'a> {\n    pub prompt: &'a str,\n    pub skill_prompt: Option<&'a str>,\n    pub role: Role,\n''',
        "add capability skill prompt field",
    )
    replace_once(
        capability,
        '''        Self {\n            prompt,\n            role,\n''',
        '''        Self {\n            prompt,\n            skill_prompt: None,\n            role,\n''',
        "default capability skill prompt",
    )
    replace_once(
        capability,
        '''    pub fn with_context_token_cap(mut self, cap: usize) -> Self {\n''',
        '''    pub fn with_skill_prompt(mut self, prompt: &'a str) -> Self {\n        self.skill_prompt = Some(prompt);\n        self\n    }\n\n    pub fn with_context_token_cap(mut self, cap: usize) -> Self {\n''',
        "add capability skill prompt builder",
    )
    replace_once(
        capability,
        '''    let context_request = ContextPacketRequest::new(request.prompt)\n        .with_role(request.role)\n        .with_token_cap(request.context_token_cap)\n        .with_skills(request.include_skills);\n\n    CapabilitySelection {\n''',
        '''    let mut context_request = ContextPacketRequest::new(request.prompt)\n        .with_role(request.role)\n        .with_token_cap(request.context_token_cap)\n        .with_skills(request.include_skills);\n    if let Some(skill_prompt) = request.skill_prompt {\n        context_request = context_request.with_skill_prompt(skill_prompt);\n    }\n\n    CapabilitySelection {\n''',
        "forward capability skill prompt",
    )

    controller = CRATE / "src/native_extensions/graph/controller.rs"
    replace_once(
        controller,
        '''                    let context_query = crate::native_extensions::ecosystem::WorkerContextQuery {\n                        role: Some(role),\n                        node_objective: briefing.clone(),\n                        graph_goal: self.options.goal.clone(),\n                        target_hints: task.focus.clone().into_iter().collect(),\n                        failure_hint: None,\n                    }\n                    .render();\n                    crate::native_extensions::ecosystem::select_capabilities(\n''',
        '''                    let worker_context_query =\n                        crate::native_extensions::ecosystem::WorkerContextQuery {\n                            role: Some(role),\n                            node_objective: briefing.clone(),\n                            graph_goal: self.options.goal.clone(),\n                            target_hints: task.focus.clone().into_iter().collect(),\n                            failure_hint: None,\n                        };\n                    let context_query = worker_context_query.render();\n                    let skill_query = worker_context_query.render_skill_query();\n                    crate::native_extensions::ecosystem::select_capabilities(\n''',
        "separate graph context and skill queries",
    )
    replace_once(
        controller,
        '''                        crate::native_extensions::ecosystem::CapabilityRequest::new(\n                            &context_query,\n                            role,\n                        )\n                        .with_context_token_cap(\n''',
        '''                        crate::native_extensions::ecosystem::CapabilityRequest::new(\n                            &context_query,\n                            role,\n                        )\n                        .with_skill_prompt(&skill_query)\n                        .with_context_token_cap(\n''',
        "pass graph skill query",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tests", action="store_true")
    parser.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests == args.impl:
        raise SystemExit("choose exactly one of --tests or --impl")
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
