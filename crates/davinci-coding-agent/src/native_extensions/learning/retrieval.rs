use std::cmp::Ordering;
use std::collections::HashSet;

use davinci_agent::{describe_skill, Skill, SkillDescriptor};

use crate::native_extensions::graph::Role;
use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningScope, SkillContextCandidate, SkillLedgerRecord,
};
use crate::native_extensions::vector_memory::cosine_similarity;

#[derive(Debug, Clone, PartialEq)]
pub struct SkillMatch {
    pub descriptor: SkillDescriptor,
    pub score: f32,
    pub scope: LearningScope,
    pub status: ArtifactStatus,
    pub verified_successes: u64,
    pub verified_failures: u64,
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

#[allow(dead_code)]
pub fn rank_skills(
    query: &str,
    skills: &[Skill],
    ledger: &[SkillLedgerRecord],
    limit: usize,
) -> Vec<SkillMatch> {
    rank_skills_with_embeddings(query, None, skills, None, ledger, limit)
}

pub fn rank_skills_with_embeddings(
    query: &str,
    query_embedding: Option<&[f32]>,
    skills: &[Skill],
    skill_embeddings: Option<&[Option<Vec<f32>>]>,
    ledger: &[SkillLedgerRecord],
    limit: usize,
) -> Vec<SkillMatch> {
    let query_tokens = tokenize(query);

    let mut matches = Vec::new();

    for (idx, skill) in skills.iter().enumerate() {
        let record = ledger.iter().find(|r| r.name == skill.name);

        let status = record.map(|r| r.status).unwrap_or(ArtifactStatus::Active);
        if status == ArtifactStatus::Archived || status == ArtifactStatus::Rejected {
            continue;
        }

        let scope = record.map(|r| r.scope).unwrap_or_else(|| {
            let path_str = skill.path.to_string_lossy();
            if path_str.contains(".davinci") || path_str.contains(".pi") {
                LearningScope::Project
            } else {
                LearningScope::Global
            }
        });

        let successes = record.map(|r| r.success_count).unwrap_or(0);
        let failures = record.map(|r| r.failure_count).unwrap_or(0);

        let (text_score, is_match) = if query_tokens.is_empty() {
            (0.5, true)
        } else {
            let name_tokens = tokenize(&skill.name);
            let desc_tokens = tokenize(&skill.description);

            let name_overlap = query_tokens.intersection(&name_tokens).count() as f32;
            let desc_overlap = query_tokens.intersection(&desc_tokens).count() as f32;

            let name_score = name_overlap / query_tokens.len() as f32;
            let desc_score = desc_overlap / query_tokens.len() as f32;

            let mut base = 0.6 * name_score + 0.4 * desc_score;

            // Extra bonus if substring match
            let q_lower = query.to_lowercase();
            if skill.name.to_lowercase().contains(&q_lower) {
                base = (base + 0.3).min(1.0);
            } else if skill.description.to_lowercase().contains(&q_lower) {
                base = (base + 0.15).min(1.0);
            }

            let has_overlap = name_overlap > 0.0 || desc_overlap > 0.0;
            (base, has_overlap)
        };

        let dense_score = match (
            query_embedding,
            skill_embeddings.and_then(|se| se.get(idx).and_then(|e| e.as_ref())),
        ) {
            (Some(q_emb), Some(s_emb)) => Some(cosine_similarity(q_emb, s_emb)),
            _ => None,
        };

        let combined_text_score = if let Some(dense) = dense_score {
            0.65 * text_score + 0.35 * dense
        } else {
            text_score
        };

        let is_overall_match = is_match || dense_score.map(|d| d >= 0.5).unwrap_or(false);

        if !is_overall_match && !query.trim().is_empty() {
            continue;
        }

        let scope_boost = if scope == LearningScope::Project {
            0.08
        } else {
            0.0
        };
        let status_boost = if status == ArtifactStatus::Active {
            0.08
        } else {
            0.0
        };
        let success_boost = 0.05 * (1.0 + successes as f32).ln();
        let failure_penalty = if (successes + failures) > 0 {
            0.20 * (failures as f32 / (successes + failures) as f32)
        } else {
            0.0
        };

        let raw_score = combined_text_score * 0.75 + scope_boost + status_boost + success_boost
            - failure_penalty;
        let final_score = raw_score.clamp(0.0, 1.0);

        matches.push(SkillMatch {
            descriptor: describe_skill(skill),
            score: final_score,
            scope,
            status,
            verified_successes: successes,
            verified_failures: failures,
        });
    }

    matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.descriptor.name.cmp(&b.descriptor.name))
    });

    if limit > 0 && matches.len() > limit {
        matches.truncate(limit);
    }

    matches
}

pub fn role_bias(role: Role, skill_name: &str, skill_desc: &str) -> f32 {
    let text = format!("{} {}", skill_name, skill_desc).to_lowercase();
    let keywords: &[&str] = match role {
        Role::Classifier => &["classify", "triage", "scope", "complexity", "label"],
        Role::Researcher => &[
            "research",
            "investigate",
            "explore",
            "search",
            "docs",
            "documentation",
            "reference",
            "find",
        ],
        Role::TestAnalyzer => &[
            "test",
            "testing",
            "benchmark",
            "verify",
            "verification",
            "assert",
            "coverage",
            "reproduce",
            "baseline",
        ],
        Role::Historian => &[
            "git",
            "history",
            "commit",
            "log",
            "blame",
            "changelog",
            "diff",
            "regression",
        ],
        Role::Planner => &[
            "plan",
            "design",
            "architecture",
            "strategy",
            "roadmap",
            "spec",
            "breakdown",
        ],
        Role::Writer => &[
            "code",
            "implement",
            "refactor",
            "patch",
            "fix",
            "write",
            "build",
            "edit",
            "rust",
        ],
        Role::Reviewer => &[
            "review", "audit", "critique", "quality", "lint", "security", "syntax", "check",
        ],
    };
    if keywords.iter().any(|kw| text.contains(kw)) {
        0.12
    } else {
        0.0
    }
}

pub fn select_graph_skill_candidates(
    query: &str,
    role: Role,
    skills: &[Skill],
    ledger: &[SkillLedgerRecord],
    max_skills: usize,
    token_cap: usize,
) -> Vec<SkillContextCandidate> {
    if query.trim().is_empty() || max_skills == 0 || token_cap == 0 {
        return Vec::new();
    }

    let matches = rank_skills_with_embeddings(query, None, skills, None, ledger, 0);

    const MIN_SKILL_RELEVANCE: f32 = 0.35;

    struct ScoredCandidate<'a> {
        skill: &'a Skill,
        version: u64,
        content_hash: String,
        score: f32,
    }

    let mut candidates: Vec<ScoredCandidate<'_>> = Vec::new();

    for m in &matches {
        if m.score < MIN_SKILL_RELEVANCE {
            continue;
        }

        let Some(skill) = skills.iter().find(|s| s.name == m.descriptor.name) else {
            continue;
        };

        let record = ledger.iter().find(|r| r.name == skill.name);
        let version = record.map(|r| r.version as u64).unwrap_or(1);
        let content_hash = record
            .map(|r| r.content_hash.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| crate::native_extensions::vector_memory::content_hash(&skill.body));

        let role_boost = role_bias(role, &skill.name, &skill.description);
        let final_score = (m.score + role_boost).clamp(0.0, 1.0);

        candidates.push(ScoredCandidate {
            skill,
            version,
            content_hash,
            score: final_score,
        });
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.skill.name.cmp(&b.skill.name))
    });

    let mut accumulated_tokens = 0;
    let mut selected = Vec::new();

    for c in candidates {
        if selected.len() >= max_skills {
            break;
        }
        let est_tokens = (c.skill.body.chars().count() + 3) / 4;
        if accumulated_tokens + est_tokens > token_cap {
            let remaining = token_cap.saturating_sub(accumulated_tokens);
            if remaining > 0 {
                let char_cap = remaining.saturating_mul(4);
                let truncated: String = c.skill.body.chars().take(char_cap).collect();
                let est_trunc = (truncated.chars().count() + 3) / 4;
                selected.push(SkillContextCandidate {
                    name: c.skill.name.clone(),
                    version: c.version,
                    content_hash: c.content_hash,
                    body: truncated,
                    score: c.score,
                    estimated_tokens: est_trunc,
                });
            }
            break;
        }
        accumulated_tokens += est_tokens;
        selected.push(SkillContextCandidate {
            name: c.skill.name.clone(),
            version: c.version,
            content_hash: c.content_hash,
            body: c.skill.body.clone(),
            score: c.score,
            estimated_tokens: est_tokens,
        });
    }

    selected
}

pub fn format_skill_context(skills: &[SkillContextCandidate]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, s) in skills.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "<skill name=\"{}\" version=\"{}\">\n{}\n</skill>",
            s.name,
            s.version,
            s.body.trim()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::SkillOrigin;
    use std::path::PathBuf;

    fn fixture_skill(name: &str, description: &str, path: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: description.to_string(),
            path: PathBuf::from(path),
            body: format!("# {}\n{}", name, description),
            base_dir: PathBuf::from(path)
                .parent()
                .unwrap_or(&PathBuf::from("."))
                .to_path_buf(),
        }
    }

    fn fixture_ledger_record(
        name: &str,
        scope: LearningScope,
        status: ArtifactStatus,
        successes: u64,
        failures: u64,
    ) -> SkillLedgerRecord {
        SkillLedgerRecord {
            skill_id: format!("id-{}", name),
            name: name.to_string(),
            scope,
            origin: SkillOrigin::LearnedReview,
            status,
            path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
            content_hash: "hash".to_string(),
            version: 1,
            success_count: successes,
            failure_count: failures,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        }
    }

    #[test]
    fn rank_skills_prefers_relevant_skill() {
        let skills = vec![
            fixture_skill(
                "deploy-rust-flyio",
                "Deploy and verify Rust applications on Fly.io",
                "/global/deploy-rust-flyio/SKILL.md",
            ),
            fixture_skill(
                "debug-sqlx",
                "Diagnose SQLx compile and offline metadata failures",
                "/proj/.pi/skills/debug-sqlx/SKILL.md",
            ),
            fixture_skill(
                "release-rust-cli",
                "Prepare, verify, and publish a Rust CLI release",
                "/global/release-rust-cli/SKILL.md",
            ),
        ];

        let ledger = vec![fixture_ledger_record(
            "debug-sqlx",
            LearningScope::Project,
            ArtifactStatus::Active,
            3,
            0,
        )];

        let hits = rank_skills("fix sqlx offline compile", &skills, &ledger, 3);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].descriptor.name, "debug-sqlx");
    }

    #[test]
    fn project_scope_beats_global_at_equal_relevance() {
        let skills = vec![
            fixture_skill("skill-a", "shared procedure", "/global/skill-a/SKILL.md"),
            fixture_skill(
                "skill-b",
                "shared procedure",
                "/proj/.pi/skills/skill-b/SKILL.md",
            ),
        ];
        let ledger = vec![
            fixture_ledger_record(
                "skill-a",
                LearningScope::Global,
                ArtifactStatus::Active,
                0,
                0,
            ),
            fixture_ledger_record(
                "skill-b",
                LearningScope::Project,
                ArtifactStatus::Active,
                0,
                0,
            ),
        ];

        let hits = rank_skills("procedure", &skills, &ledger, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].descriptor.name, "skill-b");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn active_status_beats_candidate() {
        let skills = vec![
            fixture_skill("skill-a", "shared procedure", "/skills/a/SKILL.md"),
            fixture_skill("skill-b", "shared procedure", "/skills/b/SKILL.md"),
        ];
        let ledger = vec![
            fixture_ledger_record(
                "skill-a",
                LearningScope::Project,
                ArtifactStatus::Candidate,
                0,
                0,
            ),
            fixture_ledger_record(
                "skill-b",
                LearningScope::Project,
                ArtifactStatus::Active,
                0,
                0,
            ),
        ];

        let hits = rank_skills("procedure", &skills, &ledger, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].descriptor.name, "skill-b");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn failure_rate_penalizes_skill() {
        let skills = vec![
            fixture_skill("skill-a", "shared procedure", "/skills/a/SKILL.md"),
            fixture_skill("skill-b", "shared procedure", "/skills/b/SKILL.md"),
        ];
        let ledger = vec![
            fixture_ledger_record(
                "skill-a",
                LearningScope::Project,
                ArtifactStatus::Active,
                0,
                5,
            ),
            fixture_ledger_record(
                "skill-b",
                LearningScope::Project,
                ArtifactStatus::Active,
                0,
                0,
            ),
        ];

        let hits = rank_skills("procedure", &skills, &ledger, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].descriptor.name, "skill-b");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn archived_skills_are_excluded() {
        let skills = vec![
            fixture_skill("skill-a", "shared procedure", "/skills/a/SKILL.md"),
            fixture_skill("skill-b", "shared procedure", "/skills/b/SKILL.md"),
        ];
        let ledger = vec![
            fixture_ledger_record(
                "skill-a",
                LearningScope::Project,
                ArtifactStatus::Archived,
                0,
                0,
            ),
            fixture_ledger_record(
                "skill-b",
                LearningScope::Project,
                ArtifactStatus::Active,
                0,
                0,
            ),
        ];

        let hits = rank_skills("procedure", &skills, &ledger, 2);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].descriptor.name, "skill-b");
    }

    #[test]
    fn dense_skill_ranking_with_fixture_vectors() {
        let query_vec = vec![1.0, 0.0];
        let sqlx_vec = vec![0.99, 0.01];
        let deploy_vec = vec![0.0, 1.0];
        assert!(
            cosine_similarity(&query_vec, &sqlx_vec) > cosine_similarity(&query_vec, &deploy_vec)
        );

        let s1 = fixture_skill("skill-a", "database tool", "/skills/a/SKILL.md");
        let s2 = fixture_skill("skill-b", "database tool", "/skills/b/SKILL.md");
        let skills = vec![s1, s2];
        let embeddings = vec![Some(sqlx_vec), Some(deploy_vec)];
        let matches = rank_skills_with_embeddings(
            "database",
            Some(&query_vec),
            &skills,
            Some(&embeddings),
            &[],
            10,
        );
        assert_eq!(matches[0].descriptor.name, "skill-a");
    }

    #[test]
    fn graph_skill_context_respects_max_count_and_token_cap() {
        let mut skills = Vec::new();
        for i in 1..=5 {
            let body = format!(
                "# procedure-{}\nDetail content for procedure {}.\n{}",
                i,
                i,
                "procedure detail step text ".repeat(80)
            );
            skills.push(Skill {
                name: format!("procedure-{}", i),
                description: "Procedure for test verification".into(),
                path: PathBuf::from(format!("/skills/p{}/SKILL.md", i)),
                body,
                base_dir: PathBuf::from("."),
            });
        }

        // Request max 2 skills, cap 1000 tokens
        let selected = select_graph_skill_candidates(
            "procedure verification",
            Role::TestAnalyzer,
            &skills,
            &[],
            2,
            1000,
        );
        assert_eq!(selected.len(), 2);
        let total_tokens: usize = selected.iter().map(|s| s.estimated_tokens).sum();
        assert!(total_tokens <= 1000);

        // Small token cap enforces truncation
        let tiny = select_graph_skill_candidates(
            "procedure verification",
            Role::TestAnalyzer,
            &skills,
            &[],
            2,
            50,
        );
        assert_eq!(tiny.len(), 1);
        assert!(tiny[0].estimated_tokens <= 50);
    }

    #[test]
    fn graph_skill_context_omits_irrelevant_skills() {
        let skills = vec![
            fixture_skill(
                "deploy-flyio",
                "Deploy app to flyio cloud",
                "/skills/deploy/SKILL.md",
            ),
            fixture_skill(
                "baking-bread",
                "Baking sourdough bread at home",
                "/skills/bread/SKILL.md",
            ),
            fixture_skill(
                "gardening-tips",
                "Pruning rose bushes and trees",
                "/skills/garden/SKILL.md",
            ),
        ];

        let selected =
            select_graph_skill_candidates("flyio deployment", Role::Writer, &skills, &[], 2, 1000);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "deploy-flyio");
    }

    #[test]
    fn graph_skill_context_empty_on_blank_query() {
        let skills = vec![fixture_skill(
            "deploy-flyio",
            "Deploy app to flyio cloud",
            "/skills/deploy/SKILL.md",
        )];

        let selected = select_graph_skill_candidates("   ", Role::Writer, &skills, &[], 2, 1000);
        assert!(selected.is_empty());
    }

    #[test]
    fn graph_skill_context_role_compatibility_bias() {
        let skills = vec![
            fixture_skill(
                "pipeline-audit",
                "review and audit pull request quality",
                "/skills/audit/SKILL.md",
            ),
            fixture_skill(
                "pipeline-patch",
                "write and patch code defects",
                "/skills/patch/SKILL.md",
            ),
        ];

        let rev_selected =
            select_graph_skill_candidates("pipeline", Role::Reviewer, &skills, &[], 2, 1000);
        assert_eq!(rev_selected[0].name, "pipeline-audit");

        let wrt_selected =
            select_graph_skill_candidates("pipeline", Role::Writer, &skills, &[], 2, 1000);
        assert_eq!(wrt_selected[0].name, "pipeline-patch");
    }

    #[test]
    fn graph_skill_context_format_produces_valid_tags() {
        let skills = vec![SkillContextCandidate {
            name: "test-skill".into(),
            version: 2,
            content_hash: "hash123".into(),
            body: "# Test Skill\nInstructions here.".into(),
            score: 0.9,
            estimated_tokens: 10,
        }];
        let formatted = format_skill_context(&skills);
        assert!(formatted.starts_with("<skill name=\"test-skill\" version=\"2\">"));
        assert!(formatted.ends_with("</skill>"));
        assert!(formatted.contains("Instructions here."));
    }
}
