use std::cmp::Ordering;
use std::collections::HashSet;

use davinci_agent::{describe_skill, Skill, SkillDescriptor};

use crate::native_extensions::learning::types::{ArtifactStatus, LearningScope, SkillLedgerRecord};
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
            if path_str.contains(".pi") {
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
}
