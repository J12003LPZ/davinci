use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Deserialize;

use crate::native_extensions::learning::config::LearningConfig;
use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningArtifact, LearningCandidate, LearningEvidence, LearningScope,
};
use crate::native_extensions::vector_memory::redact_secrets;

#[derive(Debug, Clone)]
pub struct ReviewRun {
    pub id: String,
    pub cancelled: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
}

impl ReviewRun {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            cancelled: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn mark_finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReviewResult {
    pub candidates: Vec<LearningCandidate>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureCandidate {
    #[serde(default = "default_project_scope")]
    pub scope: LearningScope,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub rationale: String,
    pub artifact: LearningArtifact,
}

fn default_project_scope() -> LearningScope {
    LearningScope::Project
}

fn default_confidence() -> f32 {
    0.85
}

#[derive(Debug, Clone, Deserialize)]
struct FixturePayload {
    #[serde(default)]
    pub candidates: Vec<FixtureCandidate>,
}

fn now_ms() -> u64 {
    crate::native_extensions::learning::types::now_ms()
}

fn redact_artifact(artifact: LearningArtifact) -> LearningArtifact {
    match artifact {
        LearningArtifact::Memory {
            memory_kind,
            text,
            importance,
        } => LearningArtifact::Memory {
            memory_kind,
            text: redact_secrets(&text),
            importance,
        },
        LearningArtifact::SkillCreate {
            name,
            description,
            body,
        } => LearningArtifact::SkillCreate {
            name,
            description: redact_secrets(&description),
            body: redact_secrets(&body),
        },
        LearningArtifact::SkillPatch {
            name,
            old_text,
            new_text,
            expected_hash,
        } => LearningArtifact::SkillPatch {
            name,
            old_text,
            new_text: redact_secrets(&new_text),
            expected_hash,
        },
        LearningArtifact::SkillSupportFile {
            name,
            relative_path,
            content,
            expected_hash,
        } => LearningArtifact::SkillSupportFile {
            name,
            relative_path,
            content: redact_secrets(&content),
            expected_hash,
        },
        LearningArtifact::FailureLesson { text, importance } => LearningArtifact::FailureLesson {
            text: redact_secrets(&text),
            importance,
        },
    }
}

pub fn parse_review_fixture(
    raw: &str,
    evidence: &LearningEvidence,
    max_candidates: usize,
) -> Result<ReviewResult, String> {
    let parsed: FixturePayload =
        serde_json::from_str(raw).map_err(|e| format!("malformed review fixture: {}", e))?;

    let mut candidates = Vec::new();
    let now = now_ms();

    for (idx, fixture) in parsed.candidates.into_iter().enumerate() {
        if candidates.len() >= max_candidates {
            break;
        }

        let redacted_rationale = redact_secrets(&fixture.rationale);
        let redacted_art = redact_artifact(fixture.artifact);

        let candidate = LearningCandidate {
            id: format!("cand-{}-{}", evidence.turn, idx + 1),
            scope: fixture.scope,
            status: ArtifactStatus::Candidate,
            artifact: redacted_art,
            confidence: fixture.confidence,
            source_session_id: evidence.session_id.clone(),
            source_repo_id: evidence.repo_id.clone(),
            source_turn: evidence.turn,
            created_at_ms: now,
            evidence: evidence.verification.clone(),
            rationale: redacted_rationale,
        };
        candidates.push(candidate);
    }

    Ok(ReviewResult {
        candidates,
        diagnostics: Vec::new(),
    })
}

#[allow(dead_code)]
pub struct LearningReviewer;

#[allow(dead_code)]
impl LearningReviewer {
    pub fn spawn_review(
        evidence: LearningEvidence,
        config: LearningConfig,
        run: ReviewRun,
    ) -> std::thread::JoinHandle<ReviewResult> {
        std::thread::Builder::new()
            .name(format!("learning-review-{}", run.id))
            .spawn(move || execute_review(&evidence, &config, &run))
            .expect("failed to spawn learning review thread")
    }

    pub fn cancel_review(run: &ReviewRun) {
        run.cancel();
    }
}

#[allow(dead_code)]
pub fn spawn_review(
    evidence: LearningEvidence,
    config: LearningConfig,
    run: ReviewRun,
) -> std::thread::JoinHandle<ReviewResult> {
    LearningReviewer::spawn_review(evidence, config, run)
}

#[allow(dead_code)]
pub fn cancel_review(run: &ReviewRun) {
    LearningReviewer::cancel_review(run);
}

pub fn execute_review(
    evidence: &LearningEvidence,
    config: &LearningConfig,
    run: &ReviewRun,
) -> ReviewResult {
    if run.is_cancelled() {
        return ReviewResult {
            candidates: Vec::new(),
            diagnostics: vec!["review cancelled before start".into()],
        };
    }

    if std::env::var("PI_LEARNING_DISABLE_BACKGROUND")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        run.mark_finished();
        return ReviewResult {
            candidates: Vec::new(),
            diagnostics: vec![
                "background review disabled by PI_LEARNING_DISABLE_BACKGROUND".into(),
            ],
        };
    }

    // Check for fixture hook first
    if let Ok(fixture_str) = std::env::var("PI_LEARNING_REVIEW_FIXTURE") {
        match parse_review_fixture(&fixture_str, evidence, config.max_candidates_per_review) {
            Ok(result) => {
                run.mark_finished();
                return result;
            }
            Err(err) => {
                run.mark_finished();
                return ReviewResult {
                    candidates: Vec::new(),
                    diagnostics: vec![err],
                };
            }
        }
    }

    // When no fixture and no live provider hook in shadow test, fail open safely
    run.mark_finished();
    ReviewResult {
        candidates: Vec::new(),
        diagnostics: vec!["no reviewer fixture or provider configured".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::VerificationEvidence;
    use serde_json::json;
    use std::sync::Mutex;

    static FIXTURE_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_evidence() -> LearningEvidence {
        LearningEvidence {
            session_id: "sess-test".into(),
            repo_id: "repo-test".into(),
            turn: 42,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence::default(),
        }
    }

    #[test]
    fn fixture_more_than_max_candidates_is_truncated() {
        let raw = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.9,
                    "rationale": "first",
                    "artifact": {
                        "kind": "failure_lesson",
                        "text": "lesson 1",
                        "importance": 0.8
                    }
                },
                {
                    "scope": "project",
                    "confidence": 0.85,
                    "rationale": "second",
                    "artifact": {
                        "kind": "failure_lesson",
                        "text": "lesson 2",
                        "importance": 0.8
                    }
                },
                {
                    "scope": "project",
                    "confidence": 0.80,
                    "rationale": "third",
                    "artifact": {
                        "kind": "failure_lesson",
                        "text": "lesson 3",
                        "importance": 0.8
                    }
                }
            ]
        })
        .to_string();

        let evidence = fixture_evidence();
        let result = parse_review_fixture(&raw, &evidence, 2).unwrap();
        assert_eq!(result.candidates.len(), 2);
    }

    #[test]
    fn malformed_fixture_json_produces_empty_result_with_diagnostic() {
        let evidence = fixture_evidence();
        let res = parse_review_fixture("{broken-json", &evidence, 3);
        assert!(res.is_err());
    }

    #[test]
    fn fixture_redacts_secrets() {
        let raw = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.9,
                    "rationale": "Used key sk-secret12345 to fix problem",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "api-tool",
                        "description": "Bearer ghp_token12345",
                        "body": "export GITHUB_TOKEN=ghp_token12345"
                    }
                }
            ]
        })
        .to_string();

        let evidence = fixture_evidence();
        let result = parse_review_fixture(&raw, &evidence, 3).unwrap();
        assert_eq!(result.candidates.len(), 1);
        let cand = &result.candidates[0];
        assert!(!cand.rationale.contains("sk-secret12345"));
        assert!(cand.rationale.contains("[REDACTED]"));
        if let LearningArtifact::SkillCreate {
            description, body, ..
        } = &cand.artifact
        {
            assert!(!description.contains("ghp_token12345"));
            assert!(!body.contains("ghp_token12345"));
        } else {
            panic!("expected SkillCreate");
        }
    }

    #[test]
    fn review_run_cancellation() {
        let run = ReviewRun::new("run-1");
        assert!(!run.is_cancelled());
        run.cancel();
        assert!(run.is_cancelled());

        let evidence = fixture_evidence();
        let config = LearningConfig::default();
        let res = execute_review(&evidence, &config, &run);
        assert!(res.candidates.is_empty());
        assert!(res.diagnostics[0].contains("cancelled"));
    }

    #[test]
    fn execute_review_with_fixture_env() {
        let _lock = FIXTURE_ENV_LOCK.lock().unwrap();
        let fixture_json = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.95,
                    "rationale": "Learned flyio deployment",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "deploy-flyio",
                        "description": "Deploy to Fly.io",
                        "body": "fly deploy"
                    }
                }
            ]
        })
        .to_string();

        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", &fixture_json);
        let evidence = fixture_evidence();
        let config = LearningConfig::default();
        let run = ReviewRun::new("run-2");
        let res = execute_review(&evidence, &config, &run);
        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");

        assert_eq!(res.candidates.len(), 1);
        assert_eq!(res.candidates[0].source_turn, 42);
        assert!(run.is_finished());
    }

    #[test]
    fn spawn_review_thread_and_cancel_review() {
        let _lock = FIXTURE_ENV_LOCK.lock().unwrap();
        let fixture_json = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.95,
                    "rationale": "Learned flyio deployment",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "deploy-flyio",
                        "description": "Deploy to Fly.io",
                        "body": "fly deploy"
                    }
                }
            ]
        })
        .to_string();

        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", &fixture_json);
        let evidence = fixture_evidence();
        let config = LearningConfig::default();
        let run = ReviewRun::new("run-spawn");
        let handle = spawn_review(evidence, config, run.clone());
        let res = handle.join().unwrap();
        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");

        assert_eq!(res.candidates.len(), 1);
        assert!(run.is_finished());

        let run_cancel = ReviewRun::new("run-cancel-test");
        cancel_review(&run_cancel);
        assert!(run_cancel.is_cancelled());
    }

    #[test]
    fn background_review_disabled_by_env_var() {
        let _lock = FIXTURE_ENV_LOCK.lock().unwrap();
        std::env::set_var("PI_LEARNING_DISABLE_BACKGROUND", "1");
        let evidence = fixture_evidence();
        let config = LearningConfig::default();
        let run = ReviewRun::new("run-disabled");
        let res = execute_review(&evidence, &config, &run);
        std::env::remove_var("PI_LEARNING_DISABLE_BACKGROUND");

        assert!(res.candidates.is_empty());
        assert!(res.diagnostics[0].contains("disabled by PI_LEARNING_DISABLE_BACKGROUND"));
        assert!(run.is_finished());
    }
}
