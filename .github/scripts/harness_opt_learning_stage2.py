#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates/davinci-coding-agent"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


def append_once(path: Path, marker: str, addition: str) -> None:
    text = path.read_text()
    if marker in text:
        return
    path.write_text(text.rstrip() + "\n\n" + addition.strip() + "\n")


def insert_before_last_brace(path: Path, marker: str, addition: str) -> None:
    text = path.read_text()
    if marker in text:
        return
    pos = text.rfind("\n}")
    if pos < 0:
        raise SystemExit(f"{path}: final module brace not found")
    path.write_text(text[:pos] + "\n\n" + addition.strip() + "\n" + text[pos:])


def add_tests() -> None:
    types = CRATE / "src/native_extensions/learning/types.rs"
    append_once(
        types,
        "legacy_skill_metadata_defaults_applicability",
        r'''
#[cfg(test)]
mod harness_optimization_stage2_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_skill_metadata_defaults_applicability() {
        let legacy = json!({
            "skillId": "skill-legacy",
            "name": "legacy",
            "scope": "project",
            "origin": "learned_review",
            "status": "active",
            "path": "/tmp/legacy/SKILL.md",
            "contentHash": "legacy-hash",
            "version": 1,
            "successCount": 0,
            "failureCount": 0,
            "neutralCount": 0,
            "lastUsedAtMs": null,
            "createdAtMs": 1,
            "updatedAtMs": 1,
            "pinned": false
        });
        let record: SkillLedgerRecord = serde_json::from_value(legacy).unwrap();
        let serialized = serde_json::to_value(record).unwrap();
        assert!(
            serialized.get("applicability").is_some(),
            "persisted skill metadata must expose backwards-compatible applicability"
        );
    }
}
''',
    )

    retrieval = CRATE / "src/native_extensions/learning/retrieval.rs"
    append_once(
        retrieval,
        "persisted_skill_applicability_is_used_for_ranking",
        r'''
#[cfg(test)]
mod persisted_applicability_regressions {
    use super::*;
    use crate::native_extensions::learning::types::SkillOrigin;
    use serde_json::json;
    use std::path::PathBuf;

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.into(),
            description: "shared workflow rust debugging".into(),
            path: PathBuf::from(format!("/skills/{name}/SKILL.md")),
            body: "# workflow\nRun the verified workflow.".into(),
            base_dir: PathBuf::from(format!("/skills/{name}")),
        }
    }

    fn record(name: &str, applicability: serde_json::Value) -> SkillLedgerRecord {
        let base = SkillLedgerRecord {
            skill_id: format!("id-{name}"),
            name: name.into(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: PathBuf::from(format!("/skills/{name}/SKILL.md")),
            content_hash: format!("hash-{name}"),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            pinned: false,
        };
        let mut value = serde_json::to_value(base).unwrap();
        value["applicability"] = applicability;
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn persisted_skill_applicability_is_used_for_ranking() {
        let skills = vec![skill("aaa-skill"), skill("zzz-skill")];
        let ledger = vec![
            record("aaa-skill", json!({})),
            record(
                "zzz-skill",
                json!({
                    "languages": ["rust"],
                    "taskTypes": ["debugging"],
                    "pathGlobs": [],
                    "requiredSignals": [],
                    "verificationCategories": []
                }),
            ),
        ];
        let selected = select_graph_skill_candidates(
            "shared workflow rust debugging",
            Role::Writer,
            &skills,
            &ledger,
            2,
            1000,
        );
        assert_eq!(selected.len(), 2);
        assert_eq!(
            selected[0].name, "zzz-skill",
            "persisted applicability should break an otherwise lexical tie"
        );
    }
}
''',
    )

    vector = CRATE / "src/native_extensions/vector_memory.rs"
    append_once(
        vector,
        "memory_freshness_penalizes_changed_source_state",
        r'''
#[cfg(test)]
mod source_bound_freshness_regressions {
    use super::*;
    use serde_json::json;

    fn source_state_hash(path: &str, content: &[u8]) -> String {
        sha256_hex(format!("{}\0{}\n", path, sha256_hex(content)).as_bytes())
    }

    #[test]
    fn legacy_memory_record_has_neutral_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let record = json!({
            "id": "legacy-memory",
            "repoId": resolve_repo_id(dir.path()),
            "kind": "architecture",
            "text": "legacy parser architecture",
            "source": "learning",
            "contentHash": "legacy",
            "importance": 1.0,
            "createdAt": 1,
            "embedding": null,
            "confidence": 0.9,
            "sourceSessionId": null,
            "sourceTurn": null,
            "verification": null,
            "useCount": 0,
            "lastUsedAt": null
        });
        std::fs::write(mem_dir.join("records.jsonl"), format!("{}\n", record)).unwrap();
        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.0,
                ..VectorMemoryConfig::default()
            },
        );
        let hits = memory.search("legacy parser architecture", 1);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.9);
    }

    #[test]
    fn memory_freshness_penalizes_changed_source_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/auth.rs"), b"v1").unwrap();
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        let mut record = json!({
            "id": "source-memory",
            "repoId": resolve_repo_id(dir.path()),
            "kind": "architecture",
            "text": "auth parser architecture",
            "source": "learning",
            "contentHash": "source-memory",
            "importance": 1.0,
            "createdAt": 1,
            "embedding": null,
            "confidence": 0.9,
            "sourceSessionId": null,
            "sourceTurn": null,
            "verification": null,
            "useCount": 0,
            "lastUsedAt": null
        });
        record["sourcePaths"] = json!(["src/auth.rs"]);
        record["sourceStateHash"] = json!(source_state_hash("src/auth.rs", b"v1"));
        record["verifiedAtRevision"] = json!("fixture-r1");
        std::fs::write(mem_dir.join("records.jsonl"), format!("{}\n", record)).unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.0,
                ..VectorMemoryConfig::default()
            },
        );
        let before = memory.search("auth parser architecture", 1)[0].score;
        std::fs::write(dir.path().join("src/auth.rs"), b"v2").unwrap();
        let after = memory.search("auth parser architecture", 1)[0].score;
        assert!(
            after < before,
            "source-bound memory should be penalized after its verified source changes: {before} -> {after}"
        );
    }
}
''',
    )

    ecosystem = CRATE / "src/native_extensions/ecosystem/mod.rs"
    insert_before_last_brace(
        ecosystem,
        "injected_irrelevant_skill_is_not_credited_as_helpful",
        r'''
    #[test]
    fn injected_irrelevant_skill_is_not_credited_as_helpful() {
        let dir = tempdir().unwrap();
        let skills_root = dir.path().join(".pi").join("skills");
        let learning_root = dir.path().join(".pi").join("learning");
        std::fs::create_dir_all(&skills_root).unwrap();
        std::fs::create_dir_all(&learning_root).unwrap();

        let mut lines = Vec::new();
        for (name, applicability) in [
            (
                "aaa-irrelevant",
                json!({
                    "languages": ["python"],
                    "taskTypes": [],
                    "pathGlobs": [],
                    "requiredSignals": [],
                    "verificationCategories": []
                }),
            ),
            (
                "zzz-relevant",
                json!({
                    "languages": ["rust"],
                    "taskTypes": ["debugging"],
                    "pathGlobs": [],
                    "requiredSignals": [],
                    "verificationCategories": []
                }),
            ),
        ] {
            let skill_dir = skills_root.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            let skill_file = skill_dir.join("SKILL.md");
            std::fs::write(
                &skill_file,
                format!(
                    "---\nname: {name}\ndescription: shared workflow rust debugging\nroles: [writer]\n---\n# Workflow\nRun verification.\n"
                ),
            )
            .unwrap();
            lines.push(
                json!({
                    "skillId": format!("skill-{name}"),
                    "name": name,
                    "scope": "project",
                    "origin": "learned_review",
                    "status": "active",
                    "path": skill_file,
                    "contentHash": format!("hash-{name}"),
                    "version": 1,
                    "successCount": 0,
                    "failureCount": 0,
                    "neutralCount": 0,
                    "lastUsedAtMs": null,
                    "createdAtMs": 1,
                    "updatedAtMs": 1,
                    "pinned": false,
                    "applicability": applicability
                })
                .to_string(),
            );
        }
        std::fs::write(learning_root.join("skills.jsonl"), lines.join("\n") + "\n").unwrap();

        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);
        let runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "credit test".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["src/auth.rs".into()],
                    summary: "debugged auth".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });
        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, "all tests pass".into(), 5)),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(VectorMemory::new(dir.path().to_path_buf())),
            learning: Some(learning),
            governor: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };
        let run = run_graph(
            RunOptions {
                goal: "shared workflow rust debugging".into(),
                cwd: dir.path().to_path_buf(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
        );
        assert_eq!(run.phase, Phase::Done);

        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);
        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();
        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();
        assert_eq!(relevant.success_count, 1);
        assert_eq!(irrelevant.success_count, 0);
        assert_eq!(irrelevant.neutral_count, 1);
    }
''',
    )


def add_literal_defaults(text: str, struct_name: str, anchor: str, fields: str) -> str:
    # Insert fields only in struct literals. `pub <anchor>` in definitions is ignored.
    pattern = re.compile(rf"(?m)^(?P<indent>\s*){re.escape(anchor)}")
    offset = 0
    while True:
        match = pattern.search(text, offset)
        if not match:
            break
        line_start = match.start()
        prefix_line = text[line_start : text.find("\n", line_start) if "\n" in text[line_start:] else len(text)]
        if prefix_line.lstrip().startswith("pub "):
            offset = match.end()
            continue
        last_struct = text.rfind(f"{struct_name} {{", 0, line_start)
        last_end = text.rfind("};", 0, line_start)
        already = text.rfind(fields.splitlines()[0].strip(), max(0, last_struct), line_start)
        if last_struct >= 0 and last_struct > last_end and already < last_struct:
            indent = match.group("indent")
            insertion = "".join(indent + line + "\n" for line in fields.splitlines())
            text = text[:line_start] + insertion + text[line_start:]
            offset = line_start + len(insertion) + len(indent) + len(anchor)
        else:
            offset = match.end()
    return text


def add_defaults_to_literals() -> None:
    for path in list((CRATE / "src").rglob("*.rs")) + list((CRATE / "tests").rglob("*.rs")):
        text = path.read_text()
        new = add_literal_defaults(
            text,
            "SkillLedgerRecord",
            "pinned:",
            "applicability: Default::default(),",
        )
        new = add_literal_defaults(
            new,
            "MemoryRecord",
            "use_count:",
            "source_paths: Vec::new(),\nsource_state_hash: None,\nverified_at_revision: None,",
        )
        if new != text:
            path.write_text(new)


def apply_impl() -> None:
    types = CRATE / "src/native_extensions/learning/types.rs"
    text = types.read_text()
    if "pub struct SkillApplicability" not in text:
        anchor = '''#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]\n#[serde(rename_all = "snake_case")]\npub enum SkillOutcome {\n    VerifiedSuccess,\n    VerifiedFailure,\n    Neutral,\n}\n'''
        addition = anchor + r'''

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillApplicability {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub task_types: Vec<String>,
    #[serde(default)]
    pub path_globs: Vec<String>,
    #[serde(default)]
    pub required_signals: Vec<String>,
    #[serde(default)]
    pub verification_categories: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillUsageSignal {
    Injected,
    ScopeRelevant,
    VerifiedHelpful,
    VerifiedFailureRelevant,
    Neutral,
}
'''
        text = replace_once(text, anchor, addition, "learning type additions")
    if "pub applicability: SkillApplicability" not in text:
        old = '''    #[serde(default)]\n    pub last_used_at_ms: Option<u64>,\n'''
        new = '''    #[serde(default)]\n    pub applicability: SkillApplicability,\n    #[serde(default)]\n    pub last_used_at_ms: Option<u64>,\n'''
        text = replace_once(text, old, new, "skill applicability ledger field")
    types.write_text(text)

    vector = CRATE / "src/native_extensions/vector_memory.rs"
    text = vector.read_text()
    if "pub source_paths: Vec<String>" not in text:
        old = '''    #[serde(default)]\n    pub verification: Option<String>,\n    #[serde(default)]\n    pub use_count: u64,\n'''
        new = '''    #[serde(default)]\n    pub verification: Option<String>,\n    #[serde(default, skip_serializing_if = "Vec::is_empty")]\n    pub source_paths: Vec<String>,\n    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub source_state_hash: Option<String>,\n    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub verified_at_revision: Option<String>,\n    #[serde(default)]\n    pub use_count: u64,\n'''
        text = replace_once(text, old, new, "memory freshness fields")
    start_marker = "/// Deterministic compatibility factor for memories that carry an optional\n"
    if start_marker in text:
        start = text.index(start_marker)
        end = text.index("pub fn hash_to_uuid", start)
        helper = r'''pub fn source_state_hash_for_paths(cwd: &Path, paths: &[String]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let mut normalized = paths
        .iter()
        .map(|path| path.replace('\\', "/").trim_start_matches("./").to_string())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    let mut material = String::new();
    for relative in normalized {
        if relative.is_empty() {
            return None;
        }
        let bytes = fs::read(cwd.join(&relative)).ok()?;
        material.push_str(&relative);
        material.push('\0');
        material.push_str(&sha256_hex(bytes));
        material.push('\n');
    }
    Some(sha256_hex(material.as_bytes()))
}

pub fn memory_freshness(record: &MemoryRecord, cwd: &Path) -> f32 {
    if record.source_paths.is_empty() {
        return 1.0;
    }
    let authoritative = record.source == "user"
        || record.source == "user_decision"
        || matches!(record.kind, MemoryKind::Decision | MemoryKind::Constraint);
    let current = source_state_hash_for_paths(cwd, &record.source_paths);
    let factor: f32 = match (&record.source_state_hash, current) {
        (_, None) => 0.35,
        (Some(expected), Some(actual)) if expected != &actual => 0.65,
        _ => 1.0,
    };
    if authoritative {
        factor.max(0.80)
    } else {
        factor
    }
}

'''
        text = text[:start] + helper + text[end:]
    old_score = '''                let base = (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);\n                let current_state = std::env::var("DAVINCI_MEMORY_STATE_HASH").ok();\n                let score =\n                    (base * freshness_factor(record, current_state.as_deref())).clamp(0.0, 1.0);\n'''
    new_score = '''                let base =\n                    (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);\n                let score = (base * memory_freshness(record, &self.cwd)).clamp(0.0, 1.0);\n'''
    if old_score in text:
        text = replace_once(text, old_score, new_score, "source-bound freshness scoring")
    vector.write_text(text)

    add_defaults_to_literals()

    # Preserve source metadata when a record supersedes an older record.
    vector = CRATE / "src/native_extensions/vector_memory.rs"
    text = vector.read_text()
    generic = '''            source_paths: Vec::new(),\n            source_state_hash: None,\n            verified_at_revision: None,\n            use_count: 0,\n'''
    supersede_anchor = "let new_record = MemoryRecord {"
    pos = text.find(supersede_anchor)
    if pos >= 0:
        end = text.find("        };", pos)
        block = text[pos:end]
        if generic in block:
            block = block.replace(
                generic,
                '''            source_paths: prior.source_paths.clone(),\n            source_state_hash: prior.source_state_hash.clone(),\n            verified_at_revision: prior.verified_at_revision.clone(),\n            use_count: 0,\n''',
                1,
            )
            text = text[:pos] + block + text[end:]
    # Replace the obsolete preliminary freshness unit test with a legacy-neutrality check.
    old_test_start = "    #[test]\n    fn stale_memory_is_penalized_when_verified_state_changes() {"
    if old_test_start in text:
        start = text.index(old_test_start)
        end = text.index("\n    }", start) + len("\n    }")
        replacement = r'''    #[test]
    fn legacy_verification_marker_does_not_invent_staleness() {
        let record = MemoryRecord {
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
            source_paths: Vec::new(),
            source_state_hash: None,
            verified_at_revision: None,
            use_count: 0,
            last_used_at: None,
            agent_profile_name: None,
            memory_scope: None,
        };
        assert_eq!(memory_freshness(&record, Path::new(".")), 1.0);
    }'''
        text = text[:start] + replacement + text[end:]
    vector.write_text(text)

    retrieval = CRATE / "src/native_extensions/learning/retrieval.rs"
    text = retrieval.read_text()
    text = text.replace(
        "ArtifactStatus, LearningScope, SkillContextCandidate, SkillLedgerRecord,",
        "ArtifactStatus, LearningScope, SkillApplicability, SkillContextCandidate, SkillLedgerRecord,",
        1,
    )
    local_start = "#[derive(Debug, Clone, Default, PartialEq, Eq)]\npub struct SkillApplicability"
    if local_start in text:
        start = text.index(local_start)
        end = text.index("#[derive(Debug, Clone, PartialEq)]\npub struct SkillMatch", start)
        helper = r'''fn applicability_hint_matches(query: &str, hint: &str) -> bool {
    let normalized = hint
        .trim()
        .trim_matches('*')
        .replace('\\', "/")
        .to_ascii_lowercase();
    !normalized.is_empty() && query.contains(&normalized)
}

pub fn applicability_score(meta: &SkillApplicability, query: &str, _role: Role) -> f32 {
    if meta == &SkillApplicability::default() {
        return 1.0;
    }
    let query = query.replace('\\', "/").to_ascii_lowercase();
    let mut score: f32 = 1.0;
    if !meta.languages.is_empty() {
        if meta
            .languages
            .iter()
            .any(|hint| applicability_hint_matches(&query, hint))
        {
            score += 0.08;
        } else {
            score -= 0.15;
        }
    }
    if !meta.task_types.is_empty()
        && meta
            .task_types
            .iter()
            .any(|hint| applicability_hint_matches(&query, hint))
    {
        score += 0.08;
    }
    if !meta.path_globs.is_empty()
        && meta
            .path_globs
            .iter()
            .any(|hint| applicability_hint_matches(&query, hint))
    {
        score += 0.08;
    }
    if !meta.required_signals.is_empty() {
        if meta
            .required_signals
            .iter()
            .all(|hint| applicability_hint_matches(&query, hint))
        {
            score += 0.04;
        } else {
            score -= 0.20;
        }
    }
    if !meta.verification_categories.is_empty()
        && meta
            .verification_categories
            .iter()
            .any(|hint| applicability_hint_matches(&query, hint))
    {
        score += 0.04;
    }
    score.clamp(0.5, 1.2)
}

'''
        text = text[:start] + helper + text[end:]
    old = '''        let role_boost = role_bias(role, &skill.name, &skill.description);\n        let applicability =\n            applicability_score(&parse_skill_applicability(&skill.body), query, role);\n        let final_score = (m.score + role_boost) * (0.75 + 0.25 * applicability);\n        let final_score = final_score.clamp(0.0, 1.0);\n'''
    new = '''        let role_boost = role_bias(role, &skill.name, &skill.description);\n        let applicability = record\n            .map(|record| applicability_score(&record.applicability, query, role))\n            .unwrap_or(1.0);\n        let final_score = ((m.score + role_boost) * applicability).clamp(0.0, 1.0);\n'''
    if old in text:
        text = replace_once(text, old, new, "persisted applicability ranking")
    old_test = r'''    #[test]
    fn skill_applicability_biases_matching_task_without_becoming_authority() {
        let skill = fixture_skill_with_body(
            "rust-parser",
            "parser workflow",
            ".pi/skills/rust-parser/SKILL.md",
            "# rust-parser\n<!-- davinci:languages=rust;task_types=debugging;paths=crates/davinci-agent -->\nworkflow".into(),
        );
        let applicability = parse_skill_applicability(&skill.body);
        assert!(applicability.languages.contains(&"rust".to_string()));
        let matching = applicability_score(
            &applicability,
            "debug rust parser crates/davinci-agent",
            Role::Writer,
        );
        let unrelated =
            applicability_score(&applicability, "write css documentation", Role::Writer);
        assert!(matching > unrelated);
        assert!(matching <= 1.0 && unrelated >= 0.0);
    }
'''
    new_test = r'''    #[test]
    fn skill_applicability_biases_matching_task_without_becoming_authority() {
        let applicability = SkillApplicability {
            languages: vec!["rust".into()],
            task_types: vec!["debugging".into()],
            path_globs: vec!["crates/davinci-agent/**".into()],
            required_signals: Vec::new(),
            verification_categories: Vec::new(),
        };
        let matching = applicability_score(
            &applicability,
            "debugging rust crates/davinci-agent/src/lib.rs",
            Role::Writer,
        );
        let unrelated =
            applicability_score(&applicability, "write css documentation", Role::Writer);
        assert!(matching > unrelated);
        assert!(matching <= 1.2 && unrelated >= 0.5);
    }
'''
    if old_test in text:
        text = replace_once(text, old_test, new_test, "applicability unit test migration")
    retrieval.write_text(text)

    skill_manager = CRATE / "src/native_extensions/learning/skill_manager.rs"
    text = skill_manager.read_text()
    text = text.replace(
        "ArtifactStatus, LearningScope, SkillLedgerRecord, SkillOrigin,",
        "ArtifactStatus, LearningScope, SkillApplicability, SkillLedgerRecord, SkillOrigin,",
        1,
    )
    if "let applicability: SkillApplicability" not in text:
        anchor = '''        let raw_body = args\n            .get("body")\n            .and_then(Value::as_str)\n            .unwrap_or("")\n            .trim();\n'''
        addition = anchor + r'''
        let applicability: SkillApplicability = args
            .get("applicability")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| ToolError::Failed(format!("invalid applicability metadata: {e}")))?
            .unwrap_or_default();
'''
        text = replace_once(text, anchor, addition, "skill manager applicability parse")
    create_pos = text.index("        let record = SkillLedgerRecord {")
    create_end = text.index("        };", create_pos)
    create_block = text[create_pos:create_end]
    if "applicability: Default::default()," in create_block:
        create_block = create_block.replace("applicability: Default::default(),", "applicability,", 1)
        text = text[:create_pos] + create_block + text[create_end:]
    if "rec.applicability = parsed;" not in text:
        anchor = "        rec.version = updated_version;\n"
        addition = r'''        if let Some(value) = args.get("applicability") {
            let parsed: SkillApplicability = serde_json::from_value(value.clone())
                .map_err(|e| ToolError::Failed(format!("invalid applicability metadata: {e}")))?;
            rec.applicability = parsed;
        }
'''
        text = replace_once(text, anchor, addition + anchor, "skill patch applicability")
    skill_manager.write_text(text)

    learning = CRATE / "src/native_extensions/learning/mod.rs"
    text = learning.read_text()
    if "pub fn record_skill_usage_outcome" not in text:
        anchor = "    pub fn skill_view_tool(&self, _cwd: &Path, args: &Value) -> Result<ToolResult, ToolError> {\n"
        method = r'''    pub fn record_skill_usage_outcome(
        &mut self,
        skill: &SkillVersionRef,
        signal: SkillUsageSignal,
    ) -> Result<bool, String> {
        let outcome = match signal {
            SkillUsageSignal::VerifiedHelpful => SkillOutcome::VerifiedSuccess,
            SkillUsageSignal::VerifiedFailureRelevant => SkillOutcome::VerifiedFailure,
            SkillUsageSignal::Injected
            | SkillUsageSignal::ScopeRelevant
            | SkillUsageSignal::Neutral => SkillOutcome::Neutral,
        };
        self.record_skill_version_outcome(skill, outcome)
    }

'''
        text = replace_once(text, anchor, method + anchor, "skill usage outcome method")
    learning.write_text(text)

    controller = CRATE / "src/native_extensions/graph/controller.rs"
    text = controller.read_text()
    if "fn skill_scope_relevant(" not in text:
        anchor = "    pub fn record_skill_outcomes(&self, run: &GraphRun) {\n"
        helpers = r'''    fn skill_scope_relevant(
        record: &crate::native_extensions::learning::types::SkillLedgerRecord,
        task: &GraphTaskState,
        goal: &str,
    ) -> bool {
        let meta = &record.applicability;
        if meta == &crate::native_extensions::learning::types::SkillApplicability::default() {
            return false;
        }
        let mut context = goal.replace('\\', "/").to_ascii_lowercase();
        context.push(' ');
        context.push_str(&task.role.to_string().to_ascii_lowercase());
        if let Some(focus) = &task.focus {
            context.push(' ');
            context.push_str(&focus.replace('\\', "/").to_ascii_lowercase());
        }
        if let Some(mutation) = &task.mutation {
            for file in &mutation.files {
                context.push(' ');
                context.push_str(&file.path.replace('\\', "/").to_ascii_lowercase());
            }
        }
        let matches = |hint: &str| {
            let normalized = hint
                .trim()
                .trim_matches('*')
                .replace('\\', "/")
                .to_ascii_lowercase();
            !normalized.is_empty() && context.contains(&normalized)
        };
        if !meta.required_signals.is_empty()
            && !meta.required_signals.iter().all(|hint| matches(hint))
        {
            return false;
        }
        meta.languages.iter().any(|hint| matches(hint))
            || meta.task_types.iter().any(|hint| matches(hint))
            || meta.path_globs.iter().any(|hint| matches(hint))
            || meta
                .verification_categories
                .iter()
                .any(|hint| matches(hint))
    }

    fn skill_usage_signal(
        outcome: crate::native_extensions::learning::types::SkillOutcome,
        relevant: bool,
    ) -> crate::native_extensions::learning::types::SkillUsageSignal {
        use crate::native_extensions::learning::types::{SkillOutcome, SkillUsageSignal};
        match (outcome, relevant) {
            (SkillOutcome::VerifiedSuccess, true) => SkillUsageSignal::VerifiedHelpful,
            (SkillOutcome::VerifiedFailure, true) => SkillUsageSignal::VerifiedFailureRelevant,
            (SkillOutcome::Neutral, true) => SkillUsageSignal::ScopeRelevant,
            (_, false) => SkillUsageSignal::Injected,
        }
    }

'''
        text = replace_once(text, anchor, helpers + anchor, "graph skill relevance helpers")
    old_changed = '''        let changed_files: Vec<String> = run\n            .tasks\n            .iter()\n            .filter_map(|t| t.artifact_file.clone())\n            .collect();\n'''
    new_changed = '''        let changed_files: Vec<String> = run\n            .tasks\n            .iter()\n            .filter_map(|task| task.mutation.as_ref())\n            .flat_map(|mutation| mutation.files.iter().map(|file| file.path.clone()))\n            .collect();\n'''
    if old_changed in text:
        text = replace_once(text, old_changed, new_changed, "verification changed-file evidence")
    old_loop = r'''        let mut seen = std::collections::HashSet::new();
        for task in &run.tasks {
            for s in &task.skill_refs {
                let key = (s.name.clone(), s.version, s.content_hash.clone());
                if seen.insert(key) {
                    let version_ref = crate::native_extensions::learning::types::SkillVersionRef {
                        name: s.name.clone(),
                        version: s.version,
                        content_hash: s.content_hash.clone(),
                    };
                    let _ = learning.record_skill_version_outcome(&version_ref, outcome);
                }
            }
        }
'''
    new_loop = r'''        let mut seen = std::collections::HashSet::new();
        for task in &run.tasks {
            for s in &task.skill_refs {
                let key = (s.name.clone(), s.version, s.content_hash.clone());
                if seen.insert(key) {
                    let version_ref = crate::native_extensions::learning::types::SkillVersionRef {
                        name: s.name.clone(),
                        version: s.version,
                        content_hash: s.content_hash.clone(),
                    };
                    let exact_record = learning
                        .project_store
                        .skill_version(&s.name, s.version)
                        .or_else(|| learning.global_store.skill_version(&s.name, s.version))
                        .filter(|record| record.content_hash == s.content_hash)
                        .cloned();
                    let relevant = exact_record
                        .as_ref()
                        .is_some_and(|record| Self::skill_scope_relevant(record, task, &run.goal));
                    let signal = Self::skill_usage_signal(outcome, relevant);
                    let _ = learning.record_skill_usage_outcome(&version_ref, signal);
                }
            }
        }
'''
    if old_loop in text:
        text = replace_once(text, old_loop, new_loop, "scope-aware outcome credit")
    controller.write_text(text)

    # Existing full-circle behavior remains a positive-credit integration check by
    # giving its persisted skill explicit deterministic relevance metadata.
    ecosystem = CRATE / "src/native_extensions/ecosystem/mod.rs"
    text = ecosystem.read_text()
    marker = '            name: "full-circle-skill".into(),'
    pos = text.find(marker)
    if pos >= 0:
        end = text.find("        };", pos)
        block = text[pos:end]
        if "applicability: Default::default()," in block:
            block = block.replace(
                "applicability: Default::default(),",
                '''applicability: crate::native_extensions::learning::types::SkillApplicability {\n                task_types: vec!["integration".into()],\n                ..Default::default()\n            },''',
                1,
            )
            text = text[:pos] + block + text[end:]
    ecosystem.write_text(text)


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
