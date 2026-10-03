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
    rpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs"
    text = rpath.read_text()
    if "cached_semantic_skill_ranking_is_optional_and_deterministic" not in text:
        idx = text.rfind("\n}")
        tests = r'''

    #[test]
    fn cached_semantic_skill_ranking_is_optional_and_deterministic() {
        let skills = vec![
            fixture_skill("parser-fix", "repair parser state", ".pi/skills/parser/SKILL.md"),
            fixture_skill("docs", "write documentation", ".pi/skills/docs/SKILL.md"),
        ];
        let ledger = Vec::new();
        let query = vec![1.0_f32, 0.0];
        let embeddings = vec![Some(vec![1.0, 0.0]), Some(vec![0.0, 1.0])];
        let selected = select_graph_skill_candidates_with_embeddings(
            "repair state",
            Some(&query),
            &skills,
            Some(&embeddings),
            &ledger,
            Role::Writer,
            2,
            1000,
        );
        assert_eq!(selected.first().map(|s| s.name.as_str()), Some("parser-fix"));
        let lexical = select_graph_skill_candidates(
            "repair state",
            Role::Writer,
            &skills,
            &ledger,
            2,
            1000,
        );
        assert!(!lexical.is_empty());
    }

    #[test]
    fn skill_applicability_biases_matching_task_without_becoming_authority() {
        let skill = fixture_skill_with_body(
            "rust-parser",
            "parser workflow",
            ".pi/skills/rust-parser/SKILL.md",
            "# rust-parser\n<!-- davinci:languages=rust;task_types=debugging;paths=crates/davinci-agent -->\nworkflow".into(),
        );
        let applicability = parse_skill_applicability(&skill.body);
        assert!(applicability.languages.contains(&"rust".to_string()));
        let matching = applicability_score(&applicability, "debug rust parser crates/davinci-agent", Role::Writer);
        let unrelated = applicability_score(&applicability, "write css documentation", Role::Writer);
        assert!(matching > unrelated);
        assert!(matching <= 1.0 && unrelated >= 0.0);
    }
'''
        rpath.write_text(text[:idx] + tests + text[idx:])

    vpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/vector_memory.rs"
    text = vpath.read_text()
    if "stale_memory_is_penalized_when_verified_state_changes" not in text:
        idx = text.rfind("\n}")
        tests = r'''

    #[test]
    fn stale_memory_is_penalized_when_verified_state_changes() {
        let mut record = MemoryRecord {
            id: "m1".into(),
            repo_id: "repo".into(),
            kind: MemoryKind::Architecture,
            text: "parser lives in old.rs".into(),
            source: "learning".into(),
            content_hash: "h".into(),
            importance: 1.0,
            created_at: 1,
            embedding: None,
            confidence: Some(0.9),
            source_session_id: None,
            source_turn: None,
            verification: Some("state:abc".into()),
            use_count: 0,
            last_used_at: None,
            agent_profile_name: None,
            memory_scope: None,
        };
        assert_eq!(freshness_factor(&record, Some("abc")), 1.0);
        assert!(freshness_factor(&record, Some("def")) < 1.0);
        record.verification = None;
        assert_eq!(freshness_factor(&record, Some("def")), 1.0);
    }
'''
        vpath.write_text(text[:idx] + tests + text[idx:])


def apply_impl() -> None:
    rpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs"
    text = rpath.read_text()
    if "pub struct SkillApplicability" not in text:
        anchor = 'pub struct SkillMatch {\n'
        pos = text.index(anchor)
        derive_pos = text.rfind("#[derive", 0, pos)
        if derive_pos < 0:
            raise SystemExit("skill applicability insertion: SkillMatch derive not found")
        addition = r'''#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillApplicability {
    pub languages: Vec<String>,
    pub task_types: Vec<String>,
    pub path_globs: Vec<String>,
    pub required_signals: Vec<String>,
    pub verification_categories: Vec<String>,
}

pub fn parse_skill_applicability(body: &str) -> SkillApplicability {
    let Some(marker_start) = body.find("<!-- davinci:") else {
        return SkillApplicability::default();
    };
    let rest = &body[marker_start + "<!-- davinci:".len()..];
    let Some(marker_end) = rest.find("-->") else {
        return SkillApplicability::default();
    };
    let mut out = SkillApplicability::default();
    for field in rest[..marker_end].split(';') {
        let Some((key, value)) = field.split_once('=') else { continue; };
        let values = value
            .split(',')
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        match key.trim() {
            "languages" => out.languages = values,
            "task_types" => out.task_types = values,
            "paths" => out.path_globs = values,
            "required_signals" => out.required_signals = values,
            "verification" => out.verification_categories = values,
            _ => {}
        }
    }
    out
}

pub fn applicability_score(meta: &SkillApplicability, query: &str, role: Role) -> f32 {
    if meta == &SkillApplicability::default() {
        return 0.75;
    }
    let q = query.to_ascii_lowercase();
    let mut checks = 0_u32;
    let mut matches = 0_u32;
    for values in [&meta.languages, &meta.task_types, &meta.path_globs, &meta.required_signals, &meta.verification_categories] {
        if values.is_empty() { continue; }
        checks += 1;
        if values.iter().any(|value| q.contains(value)) { matches += 1; }
    }
    let role_hint = role_bias(role, "", query);
    let base = if checks == 0 { 0.75 } else { matches as f32 / checks as f32 };
    (0.65 + 0.30 * base + 0.05 * role_hint).clamp(0.0, 1.0)
}

'''
        text = text[:derive_pos] + addition + text[derive_pos:]

    old_sig = '''pub fn select_graph_skill_candidates(\n    query: &str,\n    role: Role,\n    skills: &[Skill],\n    ledger: &[SkillLedgerRecord],\n    max_skills: usize,\n    token_cap: usize,\n) -> Vec<SkillContextCandidate> {\n'''
    if "pub fn select_graph_skill_candidates_with_embeddings" not in text:
        new_sig = '''pub fn select_graph_skill_candidates(\n    query: &str,\n    role: Role,\n    skills: &[Skill],\n    ledger: &[SkillLedgerRecord],\n    max_skills: usize,\n    token_cap: usize,\n) -> Vec<SkillContextCandidate> {\n    select_graph_skill_candidates_with_embeddings(\n        query, None, skills, None, ledger, role, max_skills, token_cap,\n    )\n}\n\npub fn select_graph_skill_candidates_with_embeddings(\n    query: &str,\n    query_embedding: Option<&[f32]>,\n    skills: &[Skill],\n    skill_embeddings: Option<&[Option<Vec<f32>>]>,\n    ledger: &[SkillLedgerRecord],\n    role: Role,\n    max_skills: usize,\n    token_cap: usize,\n) -> Vec<SkillContextCandidate> {\n'''
        text = replace_once(text, old_sig, new_sig, "skill selector signature")
        text = replace_once(
            text,
            '    let matches = rank_skills_with_embeddings(query, None, skills, None, ledger, 0);\n',
            '    let matches = rank_skills_with_embeddings(\n        query, query_embedding, skills, skill_embeddings, ledger, 0,\n    );\n',
            "cached embedding ranking call",
        )
        old = '        let role_boost = role_bias(role, &skill.name, &skill.description);\n        let final_score = (m.score + role_boost).clamp(0.0, 1.0);\n'
        new = '        let role_boost = role_bias(role, &skill.name, &skill.description);\n        let applicability = applicability_score(&parse_skill_applicability(&skill.body), query, role);\n        let final_score = (m.score + role_boost) * (0.75 + 0.25 * applicability);\n        let final_score = final_score.clamp(0.0, 1.0);\n'
        text = replace_once(text, old, new, "applicability ranking")
    rpath.write_text(text)

    mpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/learning/mod.rs"
    text = mpath.read_text()
    if "graph_skill_candidates_with_embeddings" not in text:
        old = '''    #[allow(dead_code)]\n    pub fn graph_skill_candidates(\n        &self,\n        query: &str,\n        role: crate::native_extensions::graph::Role,\n        max_skills: usize,\n        token_cap: usize,\n    ) -> Vec<SkillContextCandidate> {\n        let discovered = davinci_agent::discover_skills(&[\n            self.project_skills_dir.clone(),\n            self.global_skills_dir.clone(),\n        ]);\n        let mut ledger = self.project_store.skills();\n        ledger.extend(self.global_store.skills());\n        select_graph_skill_candidates(query, role, &discovered, &ledger, max_skills, token_cap)\n    }\n'''
        new = '''    #[allow(dead_code)]\n    pub fn graph_skill_candidates(\n        &self,\n        query: &str,\n        role: crate::native_extensions::graph::Role,\n        max_skills: usize,\n        token_cap: usize,\n    ) -> Vec<SkillContextCandidate> {\n        self.graph_skill_candidates_with_embeddings(\n            query, None, None, role, max_skills, token_cap,\n        )\n    }\n\n    pub fn graph_skill_candidates_with_embeddings(\n        &self,\n        query: &str,\n        query_embedding: Option<&[f32]>,\n        skill_embeddings: Option<&[Option<Vec<f32>>]>,\n        role: crate::native_extensions::graph::Role,\n        max_skills: usize,\n        token_cap: usize,\n    ) -> Vec<SkillContextCandidate> {\n        let discovered = davinci_agent::discover_skills(&[\n            self.project_skills_dir.clone(),\n            self.global_skills_dir.clone(),\n        ]);\n        let mut ledger = self.project_store.skills();\n        ledger.extend(self.global_store.skills());\n        select_graph_skill_candidates_with_embeddings(\n            query, query_embedding, &discovered, skill_embeddings, &ledger, role, max_skills, token_cap,\n        )\n    }\n\n    pub fn record_skill_usage_outcome(\n        &mut self,\n        skill: &SkillVersionRef,\n        surfaced: bool,\n        scope_relevant: bool,\n        outcome: SkillOutcome,\n    ) -> Result<bool, String> {\n        let effective = match outcome {\n            SkillOutcome::VerifiedSuccess if !(surfaced && scope_relevant) => SkillOutcome::Neutral,\n            other => other,\n        };\n        self.record_skill_version_outcome(skill, effective)\n    }\n'''
        text = replace_once(text, old, new, "learning graph selector")
    mpath.write_text(text)

    vpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/vector_memory.rs"
    text = vpath.read_text()
    if "pub fn freshness_factor" not in text:
        anchor = 'pub fn content_hash(text: &str) -> String {\n    sha256_hex(text.as_bytes())\n}\n'
        addition = r'''

/// Deterministic compatibility factor for memories that carry an optional
/// verified repository-state marker in `verification` as `state:<digest>`.
/// Legacy memories remain neutral (1.0); mismatched state is penalized but
/// remains retrievable because older facts can still be useful historical evidence.
pub fn freshness_factor(record: &MemoryRecord, current_state: Option<&str>) -> f32 {
    let Some(expected) = record
        .verification
        .as_deref()
        .and_then(|value| value.strip_prefix("state:"))
    else {
        return 1.0;
    };
    match current_state {
        Some(current) if current == expected => 1.0,
        Some(_) => 0.55,
        None => 0.80,
    }
}
'''
        text = replace_once(text, anchor, anchor + addition, "memory freshness helper")
        old = '                let score = (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);\n'
        new = '                let base = (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);\n                let current_state = std::env::var("DAVINCI_MEMORY_STATE_HASH").ok();\n                let score = (base * freshness_factor(record, current_state.as_deref())).clamp(0.0, 1.0);\n'
        text = replace_once(text, old, new, "memory freshness scoring")
    vpath.write_text(text)


def main() -> None:
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--tests", action="store_true")
    group.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
