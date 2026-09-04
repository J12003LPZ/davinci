use crate::native_extensions::learning::config::LearningConfig;
use crate::native_extensions::learning::skill_manager::SkillWriteOrigin;
use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningArtifact, LearningCandidate, LearningScope, SkillLedgerRecord,
    SkillOrigin,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDecision {
    KeepCandidate,
    StageForApproval,
    AutoApply,
    Reject,
}

pub fn evaluate_candidate(
    candidate: &LearningCandidate,
    config: &LearningConfig,
    project_trusted: bool,
    target_skill: Option<&SkillLedgerRecord>,
) -> CandidateDecision {
    if candidate.confidence < 0.50 {
        return CandidateDecision::Reject;
    }

    if candidate.evidence.user_corrected {
        return CandidateDecision::StageForApproval;
    }

    if candidate.evidence.permission_denied {
        return CandidateDecision::Reject;
    }

    // Procedural skills require command verification, whereas declarative memory facts
    // derived from conversation or inspection can auto-apply with sufficient confidence.
    let requires_commands = !matches!(candidate.artifact, LearningArtifact::Memory { .. });

    // Check verification
    if requires_commands && candidate.evidence.commands_ran == 0 {
        return CandidateDecision::KeepCandidate;
    }

    if requires_commands && !candidate.evidence.passed {
        return CandidateDecision::KeepCandidate;
    }

    // Must have at least 0.80 confidence for auto-promotion
    if candidate.confidence < 0.80 {
        return CandidateDecision::KeepCandidate;
    }

    // Check ownership if patching or writing support file
    match &candidate.artifact {
        LearningArtifact::SkillPatch { .. } => match target_skill {
            Some(target) => {
                if target.origin == SkillOrigin::User || target.origin == SkillOrigin::Imported {
                    return CandidateDecision::StageForApproval;
                }
                if target.status != ArtifactStatus::Active {
                    return CandidateDecision::StageForApproval;
                }
            }
            None => {
                return CandidateDecision::StageForApproval;
            }
        },
        LearningArtifact::SkillSupportFile { relative_path, .. } => {
            let normalized = relative_path.replace('\\', "/");
            if normalized.starts_with("scripts/") {
                return CandidateDecision::StageForApproval;
            }
            match target_skill {
                Some(target) => {
                    if target.origin == SkillOrigin::User || target.origin == SkillOrigin::Imported
                    {
                        return CandidateDecision::StageForApproval;
                    }
                    if target.status != ArtifactStatus::Active {
                        return CandidateDecision::StageForApproval;
                    }
                }
                None => {
                    return CandidateDecision::StageForApproval;
                }
            }
        }
        _ => {}
    }

    // Scope and trust gates
    match candidate.scope {
        LearningScope::Project => {
            if !project_trusted {
                return CandidateDecision::StageForApproval;
            }
            if config.shadow_mode || !config.auto_apply_project {
                return CandidateDecision::StageForApproval;
            }
        }
        LearningScope::Global => {
            if config.shadow_mode || !config.auto_apply_global {
                return CandidateDecision::StageForApproval;
            }
        }
    }

    CandidateDecision::AutoApply
}

#[allow(dead_code)]
pub fn may_auto_apply(
    candidate: &LearningCandidate,
    config: &LearningConfig,
    project_trusted: bool,
    target_skill: Option<&SkillLedgerRecord>,
) -> bool {
    matches!(
        evaluate_candidate(candidate, config, project_trusted, target_skill),
        CandidateDecision::AutoApply
    )
}

#[allow(dead_code)]
pub fn may_patch_skill(
    skill: &SkillLedgerRecord,
    origin: SkillWriteOrigin,
    project_trusted: bool,
) -> bool {
    match origin {
        SkillWriteOrigin::ForegroundUserDirected => true,
        SkillWriteOrigin::BackgroundReview => {
            (skill.origin == SkillOrigin::LearnedReview
                || skill.origin == SkillOrigin::LearnedForeground)
                && skill.status == ArtifactStatus::Active
                && (skill.scope != LearningScope::Project || project_trusted)
        }
    }
}

#[allow(dead_code)]
pub fn verified_use_threshold_met(skill: &SkillLedgerRecord, config: &LearningConfig) -> bool {
    skill.success_count >= config.auto_promote_verified_uses && skill.failure_count == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::{ArtifactStatus, VerificationEvidence};
    use std::path::PathBuf;

    fn fixture_candidate(
        scope: LearningScope,
        artifact: LearningArtifact,
        confidence: f32,
        evidence: VerificationEvidence,
    ) -> LearningCandidate {
        LearningCandidate {
            id: "cand-test".into(),
            scope,
            status: ArtifactStatus::Candidate,
            artifact,
            confidence,
            source_session_id: "sess-1".into(),
            source_repo_id: "repo-1".into(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence,
            rationale: "test rationale".into(),
        }
    }

    fn fixture_skill_record(origin: SkillOrigin, scope: LearningScope) -> SkillLedgerRecord {
        SkillLedgerRecord {
            skill_id: "skill-1".into(),
            name: "test-skill".into(),
            scope,
            origin,
            status: ArtifactStatus::Active,
            path: PathBuf::from("/path/SKILL.md"),
            content_hash: "hash".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        }
    }

    #[test]
    fn graph_pass_with_real_command_can_promote_owned_project_skill() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::AutoApply);
    }

    #[test]
    fn nothing_ran_can_never_promote() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: None,
                commands_ran: 0, // nothing ran!
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::KeepCandidate);
    }

    #[test]
    fn graph_failure_can_never_increment_success() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: false, // failed!
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::KeepCandidate);
    }

    #[test]
    fn user_correction_blocks_auto_promotion() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: true, // user corrected!
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn untrusted_project_blocks_auto_write() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, false, None); // untrusted project!
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn global_auto_write_is_off_when_disabled() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            auto_apply_global: false,
            ..Default::default()
        };
        assert!(!config.auto_apply_global);

        let candidate = fixture_candidate(
            LearningScope::Global,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn global_auto_write_applies_by_default() {
        let config = LearningConfig::default();
        assert!(config.auto_apply_global);

        let candidate = fixture_candidate(
            LearningScope::Global,
            LearningArtifact::SkillCreate {
                name: "debug-sqlx".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::AutoApply);
    }

    #[test]
    fn memory_fact_auto_applies_without_command_execution() {
        let config = LearningConfig::default();
        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::Memory {
                memory_kind: "convention".into(),
                text: "always use forward slashes in config".into(),
                importance: 0.9,
            },
            0.88,
            VerificationEvidence {
                graph_run_id: None,
                commands_ran: 0,
                passed: false,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::AutoApply);
    }

    #[test]
    fn background_review_cannot_patch_user_skill() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillPatch {
                name: "user-skill".into(),
                old_text: "old".into(),
                new_text: "new".into(),
                expected_hash: "hash".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let user_skill = fixture_skill_record(SkillOrigin::User, LearningScope::Project);
        let decision = evaluate_candidate(&candidate, &config, true, Some(&user_skill));
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn background_review_patch_without_target_skill_staged_for_approval() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillPatch {
                name: "missing-skill".into(),
                old_text: "old".into(),
                new_text: "new".into(),
                expected_hash: "hash".into(),
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let decision = evaluate_candidate(&candidate, &config, true, None);
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn background_review_support_file_scripts_staged_for_approval() {
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let candidate = fixture_candidate(
            LearningScope::Project,
            LearningArtifact::SkillSupportFile {
                name: "learned-skill".into(),
                relative_path: "scripts/run.sh".into(),
                content: "echo hi".into(),
                expected_hash: None,
            },
            0.92,
            VerificationEvidence {
                graph_run_id: Some("run-1".into()),
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        );

        let learned_skill =
            fixture_skill_record(SkillOrigin::LearnedReview, LearningScope::Project);
        let decision = evaluate_candidate(&candidate, &config, true, Some(&learned_skill));
        assert_eq!(decision, CandidateDecision::StageForApproval);
    }

    #[test]
    fn verified_use_threshold_requires_zero_failures() {
        let config = LearningConfig::default();
        let mut skill = fixture_skill_record(SkillOrigin::LearnedReview, LearningScope::Project);
        skill.success_count = 2;
        skill.failure_count = 0;
        assert!(verified_use_threshold_met(&skill, &config));

        skill.failure_count = 1;
        assert!(!verified_use_threshold_met(&skill, &config));
    }
}
