pub mod config;
pub mod evidence;
pub mod policy;
pub mod prompts;
pub mod retrieval;
pub mod reviewer;
pub mod skill_manager;
pub mod store;
pub mod types;

pub use config::*;
pub use evidence::*;
pub use policy::*;
pub use prompts::*;
pub use retrieval::*;
pub use reviewer::*;
pub use skill_manager::*;
pub use store::*;
pub use types::*;

use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::native_extensions::vector_memory::content_hash;

#[derive(Debug, Clone)]
pub struct LearningController {
    pub config: LearningConfig,
    pub project_store: LearningStore,
    pub global_store: LearningStore,
    pub stats: LearningStats,
    pub diagnostics: Vec<String>,
    pub notifications: Vec<String>,
    pub project_skills_dir: PathBuf,
    pub global_skills_dir: PathBuf,
    pub read_set: Arc<Mutex<ReviewReadSet>>,
    pub project_trusted: bool,
    pub active_review: Option<ReviewRun>,
}

impl LearningController {
    pub fn new(cwd: &Path, agent_dir: Option<&Path>, config: Option<LearningConfig>) -> Self {
        let config = config.unwrap_or_default();
        let mut diagnostics = Vec::new();

        let project_root = cwd.join(".pi").join("learning");
        let project_store = match LearningStore::open(project_root) {
            Ok(store) => store,
            Err(err) => {
                diagnostics.push(format!("failed to open project learning store: {}", err));
                let temp = std::env::temp_dir().join("davinci_learning_fallback_project");
                LearningStore::open(temp)
                    .unwrap_or_else(|_| LearningStore::open(PathBuf::from(".")).unwrap())
            }
        };

        let global_root = agent_dir
            .map(|dir| dir.join("learning"))
            .unwrap_or_else(|| {
                if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                    PathBuf::from(home)
                        .join(".pi")
                        .join("agent")
                        .join("learning")
                } else {
                    PathBuf::from(".pi").join("learning")
                }
            });
        let global_store = match LearningStore::open(global_root) {
            Ok(store) => store,
            Err(err) => {
                diagnostics.push(format!("failed to open global learning store: {}", err));
                let temp = std::env::temp_dir().join("davinci_learning_fallback_global");
                LearningStore::open(temp)
                    .unwrap_or_else(|_| LearningStore::open(PathBuf::from(".")).unwrap())
            }
        };

        let project_skills_dir = cwd.join(".pi").join("skills");
        let global_skills_dir = agent_dir.map(|dir| dir.join("skills")).unwrap_or_else(|| {
            if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                PathBuf::from(home).join(".pi").join("agent").join("skills")
            } else {
                PathBuf::from(".pi").join("skills")
            }
        });

        Self {
            config,
            project_store,
            global_store,
            stats: LearningStats::default(),
            diagnostics,
            notifications: Vec::new(),
            project_skills_dir,
            global_skills_dir,
            read_set: Arc::new(Mutex::new(ReviewReadSet::new())),
            project_trusted: false,
            active_review: None,
        }
    }

    pub fn set_project_trusted(&mut self, trusted: bool) {
        self.project_trusted = trusted;
    }

    pub fn cancel_active_review(&mut self) {
        if let Some(run) = self.active_review.take() {
            if !run.is_finished() {
                run.cancel();
                self.stats.reviews_cancelled += 1;
                self.notifications
                    .push("learning · review cancelled by new turn".to_string());
            }
        }
    }

    pub fn drain_notifications(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notifications)
    }

    pub fn review_settled_turn(&mut self, evidence: LearningEvidence) -> Option<String> {
        if !self.config.enabled || !self.config.background_review {
            return None;
        }

        if std::env::var("PI_LEARNING_DISABLE_BACKGROUND")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            return None;
        }

        self.cancel_active_review();

        let run = ReviewRun::new(format!("rev-{}", evidence.turn));
        let run_id = run.id.clone();
        self.active_review = Some(run.clone());
        self.stats.reviews_started += 1;

        let result = execute_review(&evidence, &self.config, &run);
        self.diagnostics.extend(result.diagnostics);

        for mut candidate in result.candidates {
            self.stats.candidates_created += 1;
            let art_name = match &candidate.artifact {
                LearningArtifact::SkillCreate { name, .. } => name.clone(),
                LearningArtifact::SkillPatch { name, .. } => name.clone(),
                LearningArtifact::SkillSupportFile { name, .. } => name.clone(),
                LearningArtifact::Memory { .. } => "memory".to_string(),
                LearningArtifact::FailureLesson { .. } => "failure-lesson".to_string(),
            };
            self.notifications
                .push(format!("learning · candidate saved: {}", art_name));

            // Task 16 Step 3: Prefer patch over duplicate skill
            if let LearningArtifact::SkillCreate { name, .. } = &candidate.artifact {
                let existing_active = self
                    .project_store
                    .skill(name)
                    .or_else(|| self.global_store.skill(name));
                if let Some(existing) = existing_active {
                    if existing.status == ArtifactStatus::Active {
                        candidate.status = ArtifactStatus::PendingApproval;
                    }
                }
            }

            let target_skill = match &candidate.artifact {
                LearningArtifact::SkillPatch { name, .. } => self
                    .project_store
                    .skill(name)
                    .or_else(|| self.global_store.skill(name)),
                _ => None,
            };

            let decision =
                evaluate_candidate(&candidate, &self.config, self.project_trusted, target_skill);

            match decision {
                CandidateDecision::AutoApply => {
                    candidate.status = ArtifactStatus::Active;
                    let store = match candidate.scope {
                        LearningScope::Project => &mut self.project_store,
                        LearningScope::Global => &mut self.global_store,
                    };
                    let _ = store.upsert_candidate(candidate.clone());

                    match &candidate.artifact {
                        LearningArtifact::SkillCreate {
                            name,
                            description,
                            body,
                        } => {
                            let args = json!({
                                "action": "create",
                                "name": name,
                                "scope": match candidate.scope {
                                    LearningScope::Project => "project",
                                    LearningScope::Global => "global",
                                },
                                "description": description,
                                "body": body,
                                "candidateId": candidate.id,
                            });
                            let read_set_snapshot = self.read_set.lock().unwrap().clone();
                            let ctx = SkillManagerContext {
                                project_skills_dir: &self.project_skills_dir,
                                global_skills_dir: &self.global_skills_dir,
                                project_store: &mut self.project_store,
                                global_store: &mut self.global_store,
                                project_trusted: self.project_trusted,
                                auto_apply_global: self.config.auto_apply_global,
                                origin: SkillWriteOrigin::BackgroundReview,
                                read_set: &read_set_snapshot,
                            };
                            if SkillManager::execute(ctx, &args).is_ok() {
                                self.stats.skills_created += 1;
                                self.stats.candidates_approved += 1;
                                self.notifications
                                    .push(format!("learning · skill activated: {}", name));
                            }
                        }
                        LearningArtifact::SkillPatch {
                            name,
                            old_text,
                            new_text,
                            expected_hash,
                        } => {
                            let args = json!({
                                "action": "patch",
                                "name": name,
                                "oldText": old_text,
                                "newText": new_text,
                                "expectedHash": expected_hash,
                                "candidateId": candidate.id,
                            });
                            let read_set_snapshot = self.read_set.lock().unwrap().clone();
                            let ctx = SkillManagerContext {
                                project_skills_dir: &self.project_skills_dir,
                                global_skills_dir: &self.global_skills_dir,
                                project_store: &mut self.project_store,
                                global_store: &mut self.global_store,
                                project_trusted: self.project_trusted,
                                auto_apply_global: self.config.auto_apply_global,
                                origin: SkillWriteOrigin::BackgroundReview,
                                read_set: &read_set_snapshot,
                            };
                            if SkillManager::execute(ctx, &args).is_ok() {
                                self.stats.skills_patched += 1;
                                self.stats.candidates_approved += 1;
                                self.notifications
                                    .push(format!("learning · skill activated: {}", name));
                            }
                        }
                        LearningArtifact::SkillSupportFile {
                            name,
                            relative_path,
                            content,
                            expected_hash,
                        } => {
                            let args = json!({
                                "action": "write_file",
                                "name": name,
                                "filePath": relative_path,
                                "content": content,
                                "expectedHash": expected_hash,
                                "candidateId": candidate.id,
                            });
                            let read_set_snapshot = self.read_set.lock().unwrap().clone();
                            let ctx = SkillManagerContext {
                                project_skills_dir: &self.project_skills_dir,
                                global_skills_dir: &self.global_skills_dir,
                                project_store: &mut self.project_store,
                                global_store: &mut self.global_store,
                                project_trusted: self.project_trusted,
                                auto_apply_global: self.config.auto_apply_global,
                                origin: SkillWriteOrigin::BackgroundReview,
                                read_set: &read_set_snapshot,
                            };
                            if SkillManager::execute(ctx, &args).is_ok() {
                                self.stats.candidates_approved += 1;
                                self.notifications.push(format!(
                                    "learning · support file written: {}/{}",
                                    name, relative_path
                                ));
                            }
                        }
                        LearningArtifact::Memory { .. }
                        | LearningArtifact::FailureLesson { .. } => {
                            self.stats.candidates_approved += 1;
                        }
                    }
                }
                CandidateDecision::StageForApproval => {
                    candidate.status = ArtifactStatus::PendingApproval;
                    let store = match candidate.scope {
                        LearningScope::Project => &mut self.project_store,
                        LearningScope::Global => &mut self.global_store,
                    };
                    let _ = store.upsert_candidate(candidate);
                }
                CandidateDecision::KeepCandidate => {
                    candidate.status = ArtifactStatus::Candidate;
                    let store = match candidate.scope {
                        LearningScope::Project => &mut self.project_store,
                        LearningScope::Global => &mut self.global_store,
                    };
                    let _ = store.upsert_candidate(candidate);
                }
                CandidateDecision::Reject => {
                    candidate.status = ArtifactStatus::Rejected;
                    let store = match candidate.scope {
                        LearningScope::Project => &mut self.project_store,
                        LearningScope::Global => &mut self.global_store,
                    };
                    let _ = store.upsert_candidate(candidate);
                    self.stats.candidates_rejected += 1;
                }
            }
        }

        self.stats.reviews_completed += 1;
        Some(run_id)
    }

    pub fn skill_list_tool(&self, cwd: &Path, args: &Value) -> Result<ToolResult, ToolError> {
        self.skill_list_tool_with_query_embedding(cwd, args, None)
    }

    pub fn skill_list_tool_with_query_embedding(
        &self,
        _cwd: &Path,
        args: &Value,
        query_embedding: Option<&[f32]>,
    ) -> Result<ToolResult, ToolError> {
        let query = args.get("query").and_then(Value::as_str).unwrap_or("");
        let scope_filter = args.get("scope").and_then(Value::as_str).unwrap_or("all");
        let status_filter = args.get("status").and_then(Value::as_str).unwrap_or("all");
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 20) as usize;

        let discovered = davinci_agent::discover_skills(&[
            self.project_skills_dir.clone(),
            self.global_skills_dir.clone(),
        ]);

        let mut ledger = self.project_store.skills();
        ledger.extend(self.global_store.skills());

        let mut matches = rank_skills_with_embeddings(
            query,
            query_embedding,
            &discovered,
            None,
            &ledger,
            limit * 2,
        );

        if scope_filter == "project" {
            matches.retain(|m| m.scope == LearningScope::Project);
        } else if scope_filter == "global" {
            matches.retain(|m| m.scope == LearningScope::Global);
        }

        if status_filter != "all" {
            matches.retain(|m| {
                let status_str = match m.status {
                    ArtifactStatus::Candidate => "candidate",
                    ArtifactStatus::PendingApproval => "pending_approval",
                    ArtifactStatus::Active => "active",
                    ArtifactStatus::Archived => "archived",
                    ArtifactStatus::Rejected => "rejected",
                };
                status_str == status_filter
            });
        }

        if matches.len() > limit {
            matches.truncate(limit);
        }

        let skills_json = matches
            .into_iter()
            .map(|m| {
                json!({
                    "name": m.descriptor.name,
                    "description": m.descriptor.description,
                    "scope": match m.scope {
                        LearningScope::Project => "project",
                        LearningScope::Global => "global",
                    },
                    "status": match m.status {
                        ArtifactStatus::Candidate => "candidate",
                        ArtifactStatus::PendingApproval => "pending_approval",
                        ArtifactStatus::Active => "active",
                        ArtifactStatus::Archived => "archived",
                        ArtifactStatus::Rejected => "rejected",
                    },
                    "verifiedSuccesses": m.verified_successes,
                    "verifiedFailures": m.verified_failures,
                    "score": (m.score * 100.0).round() / 100.0,
                })
            })
            .collect::<Vec<_>>();

        let body = json!({
            "skills": skills_json,
        });

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }

    pub fn skill_view_tool(&self, _cwd: &Path, args: &Value) -> Result<ToolResult, ToolError> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing required argument 'name'".into()))?;
        let file_req = args
            .get("file")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("SKILL.md");

        let validated_rel = validate_relative_support_path(file_req).map_err(ToolError::Failed)?;

        let (skill_dir, scope, origin) = if let Some(rec) = self.project_store.skill(name) {
            let dir = rec.path.parent().unwrap_or(&rec.path).to_path_buf();
            (dir, LearningScope::Project, rec.origin)
        } else if let Some(rec) = self.global_store.skill(name) {
            let dir = rec.path.parent().unwrap_or(&rec.path).to_path_buf();
            (dir, LearningScope::Global, rec.origin)
        } else if self.project_skills_dir.join(name).exists() {
            (
                self.project_skills_dir.join(name),
                LearningScope::Project,
                SkillOrigin::User,
            )
        } else if self.global_skills_dir.join(name).exists() {
            (
                self.global_skills_dir.join(name),
                LearningScope::Global,
                SkillOrigin::User,
            )
        } else {
            return Err(ToolError::Failed(format!("skill '{}' not found", name)));
        };

        let target_path = skill_dir.join(&validated_rel);
        if !target_path.exists() {
            return Err(ToolError::Failed(format!(
                "file {:?} not found for skill {}",
                file_req, name
            )));
        }

        if let Ok(canon_dir) = skill_dir.canonicalize() {
            if let Ok(canon_target) = target_path.canonicalize() {
                if !canon_target.starts_with(&canon_dir) {
                    return Err(ToolError::Failed("path escapes skill directory".into()));
                }
            }
        }

        let content = std::fs::read_to_string(&target_path)
            .map_err(|e| ToolError::Failed(format!("failed to read {:?}: {}", target_path, e)))?;
        let hash = content_hash(&content);

        if let Ok(mut set) = self.read_set.lock() {
            set.record(target_path.clone(), hash.clone());
        }

        let body = json!({
            "name": name,
            "file": file_req,
            "content": content,
            "contentHash": hash,
            "origin": match origin {
                SkillOrigin::User => "user",
                SkillOrigin::Imported => "imported",
                SkillOrigin::LearnedForeground => "learned_foreground",
                SkillOrigin::LearnedReview => "learned_review",
            },
            "scope": match scope {
                LearningScope::Project => "project",
                LearningScope::Global => "global",
            },
        });

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }

    pub fn skill_manage_tool(
        &mut self,
        _cwd: &Path,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let read_set_snapshot = self.read_set.lock().unwrap().clone();
        let ctx = SkillManagerContext {
            project_skills_dir: &self.project_skills_dir,
            global_skills_dir: &self.global_skills_dir,
            project_store: &mut self.project_store,
            global_store: &mut self.global_store,
            project_trusted: self.project_trusted,
            auto_apply_global: self.config.auto_apply_global,
            origin: SkillWriteOrigin::ForegroundUserDirected,
            read_set: &read_set_snapshot,
        };
        SkillManager::execute(ctx, args)
    }

    pub fn record_skill_outcome(
        &mut self,
        name: &str,
        outcome: SkillOutcome,
    ) -> Result<bool, String> {
        let p = self.project_store.record_skill_outcome(name, outcome)?;
        let g = self.global_store.record_skill_outcome(name, outcome)?;
        let modified = p || g;
        if modified {
            match outcome {
                SkillOutcome::VerifiedSuccess => self.stats.verified_skill_successes += 1,
                SkillOutcome::VerifiedFailure => self.stats.verified_skill_failures += 1,
                SkillOutcome::Neutral => {}
            }
        }
        Ok(modified)
    }

    pub fn status_command(&self) -> Value {
        let project_candidates = self.project_store.candidates().len();
        let project_active = self
            .project_store
            .skills()
            .iter()
            .filter(|s| s.status == ArtifactStatus::Active)
            .count();
        let global_candidates = self.global_store.candidates().len();
        let global_active = self
            .global_store
            .skills()
            .iter()
            .filter(|s| s.status == ArtifactStatus::Active)
            .count();
        json!({
            "enabled": self.config.enabled,
            "shadowMode": self.config.shadow_mode,
            "activeReview": self.active_review.as_ref().map(|r| !r.is_finished()).unwrap_or(false),
            "project": {
                "candidates": project_candidates,
                "activeSkills": project_active,
            },
            "global": {
                "candidates": global_candidates,
                "activeSkills": global_active,
            },
            "stats": {
                "reviewsStarted": self.stats.reviews_started,
                "reviewsCompleted": self.stats.reviews_completed,
                "reviewsCancelled": self.stats.reviews_cancelled,
                "candidatesCreated": self.stats.candidates_created,
                "candidatesApproved": self.stats.candidates_approved,
                "candidatesRejected": self.stats.candidates_rejected,
                "skillsCreated": self.stats.skills_created,
                "skillsPatched": self.stats.skills_patched,
            }
        })
    }

    pub fn pending_command(&self) -> Value {
        let mut pending = Vec::new();
        for c in self.project_store.candidates() {
            if c.status == ArtifactStatus::PendingApproval {
                pending.push(json!({
                    "id": c.id,
                    "scope": "project",
                    "confidence": c.confidence,
                    "rationale": c.rationale,
                    "artifact": c.artifact,
                }));
            }
        }
        for c in self.global_store.candidates() {
            if c.status == ArtifactStatus::PendingApproval {
                pending.push(json!({
                    "id": c.id,
                    "scope": "global",
                    "confidence": c.confidence,
                    "rationale": c.rationale,
                    "artifact": c.artifact,
                }));
            }
        }
        json!({
            "pending": pending
        })
    }

    pub fn approve_command(&mut self, args: &str) -> Result<Value, String> {
        let target = args.trim();
        if target.is_empty() {
            return Err("usage: /learning-approve <candidateId|all>".into());
        }
        let is_all = target == "all";
        let mut approved_ids = Vec::new();
        let mut errors = Vec::new();

        let to_process: Vec<LearningCandidate> = if is_all {
            self.project_store
                .candidates()
                .into_iter()
                .chain(self.global_store.candidates())
                .filter(|c| c.status == ArtifactStatus::PendingApproval)
                .collect()
        } else {
            self.project_store
                .candidate(target)
                .or_else(|| self.global_store.candidate(target))
                .cloned()
                .into_iter()
                .collect()
        };

        if to_process.is_empty() {
            return Err(format!(
                "no matching pending candidate found for '{}'",
                target
            ));
        }

        for mut candidate in to_process {
            let cid = candidate.id.clone();
            candidate.status = ArtifactStatus::Active;

            let write_res = match &candidate.artifact {
                LearningArtifact::SkillCreate {
                    name,
                    description,
                    body,
                } => {
                    let args = json!({
                        "action": "create",
                        "name": name,
                        "scope": match candidate.scope {
                            LearningScope::Project => "project",
                            LearningScope::Global => "global",
                        },
                        "description": description,
                        "body": body,
                        "candidateId": candidate.id,
                    });
                    let read_set_snapshot = self.read_set.lock().unwrap().clone();
                    let ctx = SkillManagerContext {
                        project_skills_dir: &self.project_skills_dir,
                        global_skills_dir: &self.global_skills_dir,
                        project_store: &mut self.project_store,
                        global_store: &mut self.global_store,
                        project_trusted: self.project_trusted,
                        auto_apply_global: true,
                        origin: SkillWriteOrigin::ForegroundUserDirected,
                        read_set: &read_set_snapshot,
                    };
                    SkillManager::execute(ctx, &args).map(|_| name.clone())
                }
                LearningArtifact::SkillPatch {
                    name,
                    old_text,
                    new_text,
                    expected_hash,
                } => {
                    let args = json!({
                        "action": "patch",
                        "name": name,
                        "oldText": old_text,
                        "newText": new_text,
                        "expectedHash": expected_hash,
                        "candidateId": candidate.id,
                    });
                    let read_set_snapshot = self.read_set.lock().unwrap().clone();
                    let ctx = SkillManagerContext {
                        project_skills_dir: &self.project_skills_dir,
                        global_skills_dir: &self.global_skills_dir,
                        project_store: &mut self.project_store,
                        global_store: &mut self.global_store,
                        project_trusted: self.project_trusted,
                        auto_apply_global: true,
                        origin: SkillWriteOrigin::ForegroundUserDirected,
                        read_set: &read_set_snapshot,
                    };
                    SkillManager::execute(ctx, &args).map(|_| name.clone())
                }
                LearningArtifact::SkillSupportFile {
                    name,
                    relative_path,
                    content,
                    expected_hash,
                } => {
                    let mut args = json!({
                        "action": "write_file",
                        "name": name,
                        "filePath": relative_path,
                        "content": content,
                        "candidateId": candidate.id,
                    });
                    if let Some(hash) = expected_hash {
                        args["expectedHash"] = json!(hash);
                    }
                    let read_set_snapshot = self.read_set.lock().unwrap().clone();
                    let ctx = SkillManagerContext {
                        project_skills_dir: &self.project_skills_dir,
                        global_skills_dir: &self.global_skills_dir,
                        project_store: &mut self.project_store,
                        global_store: &mut self.global_store,
                        project_trusted: self.project_trusted,
                        auto_apply_global: true,
                        origin: SkillWriteOrigin::ForegroundUserDirected,
                        read_set: &read_set_snapshot,
                    };
                    SkillManager::execute(ctx, &args).map(|_| format!("{}/{}", name, relative_path))
                }
                _ => Ok("approved".into()),
            };

            match write_res {
                Ok(name) => {
                    let store = match candidate.scope {
                        LearningScope::Project => &mut self.project_store,
                        LearningScope::Global => &mut self.global_store,
                    };
                    let _ = store.upsert_candidate(candidate);
                    self.stats.candidates_approved += 1;
                    self.notifications
                        .push(format!("learning · skill activated: {}", name));
                    approved_ids.push(cid);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", cid, e));
                }
            }
        }

        Ok(json!({
            "approved": approved_ids,
            "errors": errors,
        }))
    }

    pub fn reject_command(&mut self, args: &str) -> Result<Value, String> {
        let target = args.trim();
        if target.is_empty() {
            return Err("usage: /learning-reject <candidateId|all>".into());
        }
        let is_all = target == "all";
        let mut rejected_ids = Vec::new();

        let to_reject: Vec<LearningCandidate> = if is_all {
            self.project_store
                .candidates()
                .into_iter()
                .chain(self.global_store.candidates())
                .filter(|c| {
                    c.status == ArtifactStatus::PendingApproval
                        || c.status == ArtifactStatus::Candidate
                })
                .collect()
        } else {
            self.project_store
                .candidate(target)
                .or_else(|| self.global_store.candidate(target))
                .cloned()
                .into_iter()
                .collect()
        };

        if to_reject.is_empty() {
            return Err(format!("no matching candidate found for '{}'", target));
        }

        for mut candidate in to_reject {
            candidate.status = ArtifactStatus::Rejected;
            let cid = candidate.id.clone();
            let store = match candidate.scope {
                LearningScope::Project => &mut self.project_store,
                LearningScope::Global => &mut self.global_store,
            };
            let _ = store.upsert_candidate(candidate);
            self.stats.candidates_rejected += 1;
            rejected_ids.push(cid);
        }

        Ok(json!({
            "rejected": rejected_ids
        }))
    }

    pub fn skill_list_command(&self, args: &str) -> Result<Value, String> {
        let query = args.trim();
        let tool_args = json!({
            "query": query,
            "limit": 20
        });
        let result = self
            .skill_list_tool(Path::new("."), &tool_args)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&result.content).map_err(|e| e.to_string())
    }

    pub fn skill_view_command(&self, args: &str) -> Result<Value, String> {
        let mut parts = args.split_whitespace();
        let name = parts
            .next()
            .ok_or_else(|| "usage: /skill-view <name> [file]".to_string())?;
        let file = parts.next().unwrap_or("SKILL.md");
        let tool_args = json!({
            "name": name,
            "file": file
        });
        let result = self
            .skill_view_tool(Path::new("."), &tool_args)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&result.content).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnRequest {
    pub scope: LearningScope,
    pub instruction: String,
}

pub fn parse_learn_args(args: &str) -> Result<LearnRequest, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err("usage: /learn [--global] <instruction>".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("--global") {
        let rest = rest.trim();
        if rest.is_empty() {
            return Err("usage: /learn [--global] <instruction>".to_string());
        }
        Ok(LearnRequest {
            scope: LearningScope::Global,
            instruction: rest.to_string(),
        })
    } else {
        Ok(LearnRequest {
            scope: LearningScope::Project,
            instruction: trimmed.to_string(),
        })
    }
}

pub fn build_learn_prompt(req: &LearnRequest) -> String {
    format!(
        "{}\nTarget Scope: {}\nUser Instruction:\n{}",
        FOREGROUND_LEARN_PROMPT,
        match req.scope {
            LearningScope::Project => "project (save in project .pi/skills/)",
            LearningScope::Global => "global (save in agent ~/.pi/skills/)",
        },
        req.instruction
    )
}

impl Default for LearningController {
    fn default() -> Self {
        Self::new(Path::new("."), None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::MemoryMessage;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn skill_list_and_view_progressive_disclosure() {
        let dir = tempdir().unwrap();
        let mut controller = LearningController::new(dir.path(), None, None);
        controller.set_project_trusted(true);

        // Create a skill via skill_manage
        let create_args = json!({
            "action": "create",
            "name": "test-debug",
            "scope": "project",
            "description": "A debugging skill",
            "body": "## Instructions\nDo debugging steps."
        });
        controller
            .skill_manage_tool(dir.path(), &create_args)
            .unwrap();

        // skill_list should return descriptor, not body
        let list_args = json!({"query": "debug", "limit": 5});
        let list_res = controller.skill_list_tool(dir.path(), &list_args).unwrap();
        assert!(!list_res.is_error);
        let list_json: Value = serde_json::from_str(&list_res.content).unwrap();
        let skills = list_json["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["name"], "test-debug");
        assert_eq!(skills[0]["description"], "A debugging skill");
        assert!(skills[0].get("body").is_none());

        // skill_view returns full body and records hash
        let view_args = json!({"name": "test-debug", "file": "SKILL.md"});
        let view_res = controller.skill_view_tool(dir.path(), &view_args).unwrap();
        assert!(!view_res.is_error);
        let view_json: Value = serde_json::from_str(&view_res.content).unwrap();
        assert!(view_json["content"]
            .as_str()
            .unwrap()
            .contains("Do debugging steps."));
        let hash = view_json["contentHash"].as_str().unwrap();

        let target_file = controller
            .project_skills_dir
            .join("test-debug")
            .join("SKILL.md");
        assert!(controller
            .read_set
            .lock()
            .unwrap()
            .matches(&target_file, hash));
    }

    #[test]
    fn parse_learn_args_works() {
        assert_eq!(
            parse_learn_args("--global release Rust crates").unwrap(),
            LearnRequest {
                scope: LearningScope::Global,
                instruction: "release Rust crates".into(),
            }
        );
        assert_eq!(
            parse_learn_args("how we fixed SQLx offline mode").unwrap(),
            LearnRequest {
                scope: LearningScope::Project,
                instruction: "how we fixed SQLx offline mode".into(),
            }
        );
        assert!(parse_learn_args("").is_err());
        assert!(parse_learn_args("   ").is_err());
        assert!(parse_learn_args("--global").is_err());
    }

    #[test]
    fn learning_status_and_pending_approval() {
        let dir = tempdir().unwrap();
        let mut controller = LearningController::new(dir.path(), None, None);
        controller.set_project_trusted(true);

        let status = controller.status_command();
        assert_eq!(status["enabled"], true);
        assert_eq!(status["shadowMode"], true);

        // Stage a candidate for approval
        let cand = LearningCandidate {
            id: "cand-123".into(),
            scope: LearningScope::Project,
            status: ArtifactStatus::PendingApproval,
            artifact: LearningArtifact::SkillCreate {
                name: "pending-skill".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            confidence: 0.9,
            source_session_id: "sess-1".into(),
            source_repo_id: "repo-1".into(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence: VerificationEvidence::default(),
            rationale: "reusable".into(),
        };
        controller.project_store.upsert_candidate(cand).unwrap();

        let pending = controller.pending_command();
        assert_eq!(pending["pending"].as_array().unwrap().len(), 1);

        // Approve
        let app_res = controller.approve_command("cand-123").unwrap();
        assert_eq!(app_res["approved"].as_array().unwrap().len(), 1);
        assert!(controller
            .project_skills_dir
            .join("pending-skill")
            .join("SKILL.md")
            .exists());

        // Notifications drained
        let notifs = controller.drain_notifications();
        assert!(notifs
            .iter()
            .any(|n| n.contains("skill activated: pending-skill")));
    }

    #[test]
    fn learning_reject_command() {
        let dir = tempdir().unwrap();
        let mut controller = LearningController::new(dir.path(), None, None);
        let cand = LearningCandidate {
            id: "cand-reject".into(),
            scope: LearningScope::Project,
            status: ArtifactStatus::PendingApproval,
            artifact: LearningArtifact::SkillCreate {
                name: "bad-skill".into(),
                description: "desc".into(),
                body: "body".into(),
            },
            confidence: 0.5,
            source_session_id: "sess-1".into(),
            source_repo_id: "repo-1".into(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence: VerificationEvidence::default(),
            rationale: "bad".into(),
        };
        controller.project_store.upsert_candidate(cand).unwrap();

        let rej_res = controller.reject_command("cand-reject").unwrap();
        assert_eq!(rej_res["rejected"].as_array().unwrap().len(), 1);
        assert_eq!(
            controller
                .project_store
                .candidate("cand-reject")
                .unwrap()
                .status,
            ArtifactStatus::Rejected
        );
    }

    static E2E_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn learning_e2e_shadow_mode() {
        let _lock = E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let mut controller = LearningController::new(dir.path(), None, None);
        controller.set_project_trusted(true);

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
        let evidence = LearningEvidence {
            session_id: "sess-e2e".into(),
            repo_id: "repo-e2e".into(),
            turn: 1,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };

        let res = controller.review_settled_turn(evidence);
        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");

        assert!(res.is_some());
        let candidates = controller.project_store.candidates();
        assert_eq!(candidates.len(), 1);
        assert!(
            candidates[0].status == ArtifactStatus::Candidate
                || candidates[0].status == ArtifactStatus::PendingApproval
        );
        assert!(!controller
            .project_skills_dir
            .join("deploy-flyio")
            .join("SKILL.md")
            .exists());
    }

    #[test]
    fn learning_e2e_trusted_project_auto_apply() {
        let _lock = E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let config = LearningConfig {
            enabled: true,
            background_review: true,
            shadow_mode: false,
            auto_apply_project: true,
            auto_apply_global: false,
            ..LearningConfig::default()
        };
        let mut controller = LearningController::new(dir.path(), None, Some(config));
        controller.set_project_trusted(true);

        let fixture_json = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.95,
                    "rationale": "Learned flyio deployment",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "deploy-flyio-auto",
                        "description": "Deploy to Fly.io",
                        "body": "fly deploy"
                    }
                }
            ]
        })
        .to_string();

        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", &fixture_json);
        let evidence = LearningEvidence {
            session_id: "sess-e2e".into(),
            repo_id: "repo-e2e".into(),
            turn: 1,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };

        let res = controller.review_settled_turn(evidence);
        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");

        assert!(res.is_some());
        let skill_file = controller
            .project_skills_dir
            .join("deploy-flyio-auto")
            .join("SKILL.md");
        assert!(skill_file.exists());
        let record = controller.project_store.skill("deploy-flyio-auto").unwrap();
        assert_eq!(record.origin, SkillOrigin::LearnedReview);
        assert_eq!(record.status, ArtifactStatus::Active);
    }

    #[test]
    fn learning_e2e_negative_tests() {
        let _lock = E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let config = LearningConfig {
            enabled: true,
            background_review: true,
            shadow_mode: false,
            auto_apply_project: true,
            auto_apply_global: false,
            ..LearningConfig::default()
        };

        // 1. Untrusted project blocks write
        let mut controller = LearningController::new(dir.path(), None, Some(config.clone()));
        controller.set_project_trusted(false);

        let fixture_json = json!({
            "candidates": [
                {
                    "scope": "project",
                    "confidence": 0.95,
                    "rationale": "Learned flyio deployment",
                    "artifact": {
                        "kind": "skill_create",
                        "name": "untrusted-skill",
                        "description": "Deploy to Fly.io",
                        "body": "fly deploy"
                    }
                }
            ]
        })
        .to_string();

        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", &fixture_json);
        let evidence = LearningEvidence {
            session_id: "sess-e2e".into(),
            repo_id: "repo-e2e".into(),
            turn: 1,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };

        controller.review_settled_turn(evidence);
        assert!(!controller
            .project_skills_dir
            .join("untrusted-skill")
            .join("SKILL.md")
            .exists());
        assert_eq!(
            controller
                .project_store
                .candidates()
                .into_iter()
                .next()
                .unwrap()
                .status,
            ArtifactStatus::PendingApproval
        );

        // 2. Failed verification blocks write
        let dir2 = tempdir().unwrap();
        let mut controller2 = LearningController::new(dir2.path(), None, Some(config.clone()));
        controller2.set_project_trusted(true);
        let fail_evidence = LearningEvidence {
            session_id: "sess-e2e".into(),
            repo_id: "repo-e2e".into(),
            turn: 1,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: false,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };
        controller2.review_settled_turn(fail_evidence);
        assert!(!controller2
            .project_skills_dir
            .join("untrusted-skill")
            .join("SKILL.md")
            .exists());

        // 3. Nothing ran blocks write
        let dir3 = tempdir().unwrap();
        let mut controller3 = LearningController::new(dir3.path(), None, Some(config.clone()));
        controller3.set_project_trusted(true);
        let nothing_ran = LearningEvidence {
            session_id: "sess-e2e".into(),
            repo_id: "repo-e2e".into(),
            turn: 1,
            messages: Vec::new(),
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 0,
                passed: false,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };
        controller3.review_settled_turn(nothing_ran);
        assert!(!controller3
            .project_skills_dir
            .join("untrusted-skill")
            .join("SKILL.md")
            .exists());

        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");
    }

    #[test]
    fn learning_restart_persistence() {
        let dir = tempdir().unwrap();
        let agent_dir = tempdir().unwrap();
        {
            let mut controller = LearningController::new(dir.path(), Some(agent_dir.path()), None);
            controller.set_project_trusted(true);
            let cand = LearningCandidate {
                id: "cand-persist".into(),
                scope: LearningScope::Project,
                status: ArtifactStatus::PendingApproval,
                artifact: LearningArtifact::SkillCreate {
                    name: "persisted-skill".into(),
                    description: "desc".into(),
                    body: "body".into(),
                },
                confidence: 0.9,
                source_session_id: "sess-1".into(),
                source_repo_id: "repo-1".into(),
                source_turn: 1,
                created_at_ms: 1000,
                evidence: VerificationEvidence::default(),
                rationale: "reusable".into(),
            };
            controller.project_store.upsert_candidate(cand).unwrap();
            controller.approve_command("cand-persist").unwrap();
            assert!(controller
                .project_skills_dir
                .join("persisted-skill")
                .join("SKILL.md")
                .exists());
        }

        let controller2 = LearningController::new(dir.path(), Some(agent_dir.path()), None);
        assert!(controller2.project_store.skill("persisted-skill").is_some());
        let list = controller2.skill_list_command("persisted").unwrap();
        assert!(list["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "persisted-skill"));
    }

    #[test]
    fn test_clock_override_via_env() {
        let _guard = TEST_LOCK.lock().unwrap();
        std::env::set_var("PI_LEARNING_CLOCK_MS", "1700000000123");
        assert_eq!(
            crate::native_extensions::learning::types::now_ms(),
            1700000000123
        );
        std::env::remove_var("PI_LEARNING_CLOCK_MS");
    }

    #[test]
    fn test_one_off_task_does_not_become_a_skill() {
        let _guard = TEST_LOCK.lock().unwrap();
        let fixture = json!({
            "rationale": "one-off local variable rename; no repeatable workflow",
            "candidates": []
        });
        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", fixture.to_string());

        let dir = tempdir().unwrap();
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let mut controller = LearningController::new(dir.path(), None, Some(config));
        controller.set_project_trusted(true);

        let evidence = LearningEvidence {
            session_id: "sess-one-off".into(),
            repo_id: "repo-one-off".into(),
            turn: 1,
            messages: vec![
                MemoryMessage {
                    role: "user".into(),
                    content: "rename foo to bar".into(),
                },
                MemoryMessage {
                    role: "assistant".into(),
                    content: "renamed foo to bar".into(),
                },
            ],
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: false,
                permission_denied: false,
            },
        };

        controller.review_settled_turn(evidence);
        assert_eq!(controller.stats.skills_created, 0);
        assert_eq!(controller.stats.candidates_created, 0);
        assert_eq!(controller.project_store.candidates().len(), 0);

        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");
    }

    #[test]
    fn test_user_correction_overrides_promotion() {
        let _guard = TEST_LOCK.lock().unwrap();
        let fixture = json!({
            "candidates": [{
                "scope": "project",
                "confidence": 0.95,
                "rationale": "corrected debugging workflow",
                "artifact": {
                    "kind": "skill_create",
                    "name": "corrected-debug",
                    "description": "debug procedure with correction",
                    "body": "# Corrected Debug Procedure\nRun tests with flags"
                }
            }]
        });
        std::env::set_var("PI_LEARNING_REVIEW_FIXTURE", fixture.to_string());

        let dir = tempdir().unwrap();
        let config = LearningConfig {
            shadow_mode: false,
            auto_apply_project: true,
            ..Default::default()
        };

        let mut controller = LearningController::new(dir.path(), None, Some(config));
        controller.set_project_trusted(true);

        let evidence = LearningEvidence {
            session_id: "sess-correction".into(),
            repo_id: "repo-correction".into(),
            turn: 1,
            messages: vec![MemoryMessage {
                role: "user".into(),
                content: "don't do that, run cargo check first".into(),
            }],
            tools: Vec::new(),
            run_stats: davinci_agent::RunStats::default(),
            verification: VerificationEvidence {
                graph_run_id: None,
                commands_ran: 1,
                passed: true,
                user_accepted: false,
                user_corrected: true, // User correction signal!
                permission_denied: false,
            },
        };

        controller.review_settled_turn(evidence);
        // Because of user_corrected: true, it should NOT auto-apply, but be staged for approval
        assert_eq!(controller.stats.skills_created, 0);
        assert!(!controller
            .project_skills_dir
            .join("corrected-debug")
            .join("SKILL.md")
            .exists());

        let pending = controller.pending_command();
        assert_eq!(pending["pending"].as_array().unwrap().len(), 1);
        assert_eq!(pending["pending"][0]["artifact"]["name"], "corrected-debug");

        std::env::remove_var("PI_LEARNING_REVIEW_FIXTURE");
    }

    #[test]
    fn test_approve_skill_support_file() {
        let dir = tempdir().unwrap();
        let mut controller = LearningController::new(dir.path(), None, None);
        controller.set_project_trusted(true);

        // First create base active skill
        let create_cand = LearningCandidate {
            id: "cand-skill-base".into(),
            scope: LearningScope::Project,
            status: ArtifactStatus::PendingApproval,
            artifact: LearningArtifact::SkillCreate {
                name: "api-helper".into(),
                description: "helper skill".into(),
                body: "base content".into(),
            },
            confidence: 0.9,
            source_session_id: "s1".into(),
            source_repo_id: "r1".into(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence: VerificationEvidence::default(),
            rationale: "reusable".into(),
        };
        controller
            .project_store
            .upsert_candidate(create_cand)
            .unwrap();
        controller.approve_command("cand-skill-base").unwrap();

        // Now stage a SkillSupportFile candidate
        let file_cand = LearningCandidate {
            id: "cand-support-file".into(),
            scope: LearningScope::Project,
            status: ArtifactStatus::PendingApproval,
            artifact: LearningArtifact::SkillSupportFile {
                name: "api-helper".into(),
                relative_path: "references/schema.json".into(),
                content: "{\"version\": 1}".into(),
                expected_hash: None,
            },
            confidence: 0.9,
            source_session_id: "s1".into(),
            source_repo_id: "r1".into(),
            source_turn: 2,
            created_at_ms: 2000,
            evidence: VerificationEvidence::default(),
            rationale: "schema reference".into(),
        };
        controller
            .project_store
            .upsert_candidate(file_cand)
            .unwrap();

        let approve_res = controller.approve_command("cand-support-file").unwrap();
        assert_eq!(approve_res["approved"][0], "cand-support-file");

        let written_file = controller
            .project_skills_dir
            .join("api-helper")
            .join("references")
            .join("schema.json");
        assert!(written_file.exists());
        assert_eq!(
            std::fs::read_to_string(written_file).unwrap(),
            "{\"version\": 1}"
        );
    }
}
