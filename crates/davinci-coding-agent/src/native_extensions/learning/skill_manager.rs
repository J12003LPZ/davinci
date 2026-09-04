use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};

use crate::native_extensions::learning::store::LearningStore;
use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningScope, SkillLedgerRecord, SkillOrigin,
};
use crate::native_extensions::vector_memory::{content_hash, redact_secrets};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWriteOrigin {
    ForegroundUserDirected,
    BackgroundReview,
}

#[derive(Debug, Default, Clone)]
pub struct ReviewReadSet {
    seen: HashMap<PathBuf, String>,
}

impl ReviewReadSet {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    pub fn record(&mut self, path: PathBuf, content_hash: String) {
        if let Ok(canon) = path.canonicalize() {
            self.seen.insert(canon, content_hash);
        } else {
            self.seen.insert(path, content_hash);
        }
    }

    pub fn has_seen(&self, path: &Path) -> bool {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.seen.contains_key(&key)
    }

    pub fn matches(&self, path: &Path, content_hash: &str) -> bool {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.seen.get(&key).map(|s| s.as_str()) == Some(content_hash)
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.seen.clear();
    }
}

pub fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
            return false;
        }
    }
    true
}

pub fn validate_relative_support_path(rel_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel_path);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".into());
    }
    for comp in path.components() {
        match comp {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir => return Err("path traversal (..) is not allowed".into()),
            _ => return Err("invalid path component".into()),
        }
    }
    let normalized = rel_path.replace('\\', "/");
    if normalized.starts_with("references/")
        || normalized.starts_with("templates/")
        || normalized.starts_with("scripts/")
        || normalized == "SKILL.md"
    {
        Ok(path.to_path_buf())
    } else {
        Err("file must be SKILL.md or inside references/, templates/, or scripts/".into())
    }
}

fn now_ms() -> u64 {
    crate::native_extensions::learning::types::now_ms()
}

fn atomic_write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp_path = path.with_extension(format!("tmp.{}", now_ms()));
    let mut file = File::create(&tmp_path).map_err(|e| e.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);

    let had_dest = path.exists();
    let bak_path = path.with_extension(format!("bak.{}", now_ms()));
    if had_dest {
        fs::rename(path, &bak_path)
            .map_err(|e| format!("failed to backup existing file: {}", e))?;
    }
    if let Err(e) = fs::rename(&tmp_path, path) {
        if had_dest {
            let _ = fs::rename(&bak_path, path);
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("failed to replace file: {}", e));
    }
    if had_dest {
        let _ = fs::remove_file(&bak_path);
    }
    Ok(())
}

pub struct SkillManagerContext<'a> {
    pub project_skills_dir: &'a Path,
    pub global_skills_dir: &'a Path,
    pub project_store: &'a mut LearningStore,
    pub global_store: &'a mut LearningStore,
    pub project_trusted: bool,
    pub auto_apply_global: bool,
    pub origin: SkillWriteOrigin,
    pub read_set: &'a ReviewReadSet,
}

pub struct SkillManager;

impl SkillManager {
    pub fn execute(ctx: SkillManagerContext<'_>, args: &Value) -> Result<ToolResult, ToolError> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'action'".into()))?;
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'name'".into()))?;

        match action {
            "create" => Self::execute_create(ctx, name, args),
            "patch" => Self::execute_patch(ctx, name, args),
            "write_file" => Self::execute_write_file(ctx, name, args),
            "archive" => Self::execute_status_change(ctx, name, ArtifactStatus::Archived),
            "activate" => Self::execute_status_change(ctx, name, ArtifactStatus::Active),
            "reject" => Self::execute_status_change(ctx, name, ArtifactStatus::Rejected),
            other => Err(ToolError::Failed(format!("unknown action '{}'", other))),
        }
    }

    fn execute_create(
        ctx: SkillManagerContext<'_>,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        if !is_valid_skill_name(name) {
            return Err(ToolError::Failed(format!(
                "invalid skill name '{}': must be lowercase alphanumeric and hyphens (up to 64 chars)",
                name
            )));
        }

        let scope_str = args
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("project");
        let scope = match scope_str {
            "global" => LearningScope::Global,
            _ => LearningScope::Project,
        };

        if scope == LearningScope::Project
            && ctx.origin == SkillWriteOrigin::BackgroundReview
            && !ctx.project_trusted
        {
            return Err(ToolError::Failed(
                "untrusted project cannot receive autonomous project skill writes".into(),
            ));
        }

        if scope == LearningScope::Global
            && ctx.origin == SkillWriteOrigin::BackgroundReview
            && !ctx.auto_apply_global
        {
            return Err(ToolError::Failed(
                "global automatic writes are disabled by default".into(),
            ));
        }

        let target_root = match scope {
            LearningScope::Project => ctx.project_skills_dir,
            LearningScope::Global => ctx.global_skills_dir,
        };
        let skill_dir = target_root.join(name);
        let skill_file = skill_dir.join("SKILL.md");

        if skill_file.exists() {
            return Err(ToolError::Failed(format!(
                "skill '{}' already exists; use patch instead",
                name
            )));
        }

        let new_tokens = name
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect::<std::collections::HashSet<_>>();

        for existing in ctx
            .project_store
            .skills()
            .into_iter()
            .chain(ctx.global_store.skills())
        {
            if existing.status == ArtifactStatus::Active {
                if existing.name == name {
                    return Err(ToolError::Failed(format!(
                        "skill '{}' already exists; use patch instead",
                        name
                    )));
                }
                let existing_tokens = existing
                    .name
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|t| !t.is_empty())
                    .map(|t| t.to_lowercase())
                    .collect::<std::collections::HashSet<_>>();
                if !new_tokens.is_empty() && !existing_tokens.is_empty() {
                    let overlap = new_tokens.intersection(&existing_tokens).count() as f32;
                    let denom = new_tokens.len().max(existing_tokens.len()) as f32;
                    let sim = overlap / denom;
                    if sim >= 0.85 {
                        return Err(ToolError::Failed(format!(
                            "near duplicate of active skill '{}' (similarity {:.2}); use patch or choose a distinct name",
                            existing.name, sim
                        )));
                    }
                }
            }
        }

        let description = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let raw_body = args
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        let content = if raw_body.starts_with("---") {
            redact_secrets(raw_body)
        } else {
            let redacted = redact_secrets(raw_body);
            format!(
                "---\nname: {}\ndescription: {}\n---\n\n{}",
                name, description, redacted
            )
        };

        atomic_write_file(&skill_file, &content)
            .map_err(|e| ToolError::Failed(format!("failed to write skill file: {}", e)))?;

        let origin = match ctx.origin {
            SkillWriteOrigin::BackgroundReview => SkillOrigin::LearnedReview,
            SkillWriteOrigin::ForegroundUserDirected => SkillOrigin::LearnedForeground,
        };

        let content_hash_val = content_hash(&content);
        let record = SkillLedgerRecord {
            skill_id: format!("skill-{}", name),
            name: name.to_string(),
            scope,
            origin,
            status: ArtifactStatus::Active,
            path: skill_file.clone(),
            content_hash: content_hash_val.clone(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
            pinned: false,
        };

        let store = match scope {
            LearningScope::Project => ctx.project_store,
            LearningScope::Global => ctx.global_store,
        };
        store
            .upsert_skill(record)
            .map_err(|e| ToolError::Failed(format!("failed to update skill ledger: {}", e)))?;

        let body = json!({
            "status": "created",
            "name": name,
            "scope": scope,
            "path": skill_file.display().to_string(),
            "contentHash": content_hash_val,
        });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }

    fn execute_patch(
        ctx: SkillManagerContext<'_>,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        // Resolve skill path and scope
        let (path, scope, ledger_record) = if let Some(rec) = ctx.project_store.skill(name) {
            (rec.path.clone(), LearningScope::Project, Some(rec.clone()))
        } else if let Some(rec) = ctx.global_store.skill(name) {
            (rec.path.clone(), LearningScope::Global, Some(rec.clone()))
        } else if ctx.project_skills_dir.join(name).join("SKILL.md").exists() {
            (
                ctx.project_skills_dir.join(name).join("SKILL.md"),
                LearningScope::Project,
                None,
            )
        } else if ctx.global_skills_dir.join(name).join("SKILL.md").exists() {
            (
                ctx.global_skills_dir.join(name).join("SKILL.md"),
                LearningScope::Global,
                None,
            )
        } else {
            return Err(ToolError::Failed(format!("skill '{}' not found", name)));
        };

        let skill_origin = ledger_record
            .as_ref()
            .map(|r| r.origin)
            .unwrap_or(SkillOrigin::User);

        if ctx.origin == SkillWriteOrigin::BackgroundReview {
            if skill_origin == SkillOrigin::User || skill_origin == SkillOrigin::Imported {
                return Err(ToolError::Failed(
                    "background review cannot mutate user-origin or imported skills".into(),
                ));
            }
            if scope == LearningScope::Project && !ctx.project_trusted {
                return Err(ToolError::Failed(
                    "untrusted project cannot receive autonomous writes".into(),
                ));
            }
        }

        let current = fs::read_to_string(&path)
            .map_err(|e| ToolError::Failed(format!("failed to read {:?}: {}", path, e)))?;
        let current_hash = content_hash(&current);

        let expected_hash = args
            .get("expectedHash")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'expectedHash'".into()))?;

        if ctx.origin == SkillWriteOrigin::BackgroundReview && !ctx.read_set.has_seen(&path) {
            return Err(ToolError::Failed(
                "background review must skill_view the current file before patching".into(),
            ));
        }

        if current_hash != expected_hash {
            return Err(ToolError::Failed(
                "skill changed since review; reload with skill_view".into(),
            ));
        }

        if ctx.origin == SkillWriteOrigin::BackgroundReview
            && !ctx.read_set.matches(&path, expected_hash)
        {
            return Err(ToolError::Failed(
                "background review must skill_view the current file before patching".into(),
            ));
        }

        let old_text = args
            .get("oldText")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'oldText'".into()))?;
        let new_text = args
            .get("newText")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'newText'".into()))?;

        let match_count = current.matches(old_text).count();
        if match_count == 0 {
            return Err(ToolError::Failed("oldText not found in target file".into()));
        }
        if match_count > 1 {
            return Err(ToolError::Failed(
                "oldText matched multiple times in target file; patch must be unique".into(),
            ));
        }

        let redacted_new = redact_secrets(new_text);
        let patched = current.replacen(old_text, &redacted_new, 1);
        let new_hash = content_hash(&patched);

        // Rollback history (Task 16 Step 4): persist prior version in history dir
        let store_root = match scope {
            LearningScope::Project => ctx.project_store.root(),
            LearningScope::Global => ctx.global_store.root(),
        };
        let current_version = ledger_record.as_ref().map(|r| r.version).unwrap_or(1);
        let history_dir = store_root.join("history").join(name);
        if fs::create_dir_all(&history_dir).is_ok() {
            let history_file = history_dir.join(format!("{}.md", current_version));
            let _ = fs::write(&history_file, &current);

            // Bounded history: retain latest 5 versions sorted numerically
            if let Ok(entries) = fs::read_dir(&history_dir) {
                let mut files = entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
                    .collect::<Vec<_>>();
                files.sort_by_key(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0)
                });
                if files.len() > 5 {
                    for stale in &files[..files.len() - 5] {
                        let _ = fs::remove_file(stale);
                    }
                }
            }
        }

        // Re-verify hash right before write to prevent race with concurrent modification on disk
        let disk_content = fs::read_to_string(&path)
            .map_err(|e| ToolError::Failed(format!("failed to re-read {:?}: {}", path, e)))?;
        if content_hash(&disk_content) != current_hash {
            return Err(ToolError::Failed(
                "skill was modified concurrently on disk; reload with skill_view".into(),
            ));
        }

        atomic_write_file(&path, &patched)
            .map_err(|e| ToolError::Failed(format!("failed to write patched skill: {}", e)))?;

        let updated_version = current_version + 1;
        let mut rec = ledger_record.unwrap_or_else(|| SkillLedgerRecord {
            skill_id: format!("skill-{}", name),
            name: name.to_string(),
            scope,
            origin: match ctx.origin {
                SkillWriteOrigin::BackgroundReview => SkillOrigin::LearnedReview,
                SkillWriteOrigin::ForegroundUserDirected => SkillOrigin::LearnedForeground,
            },
            status: ArtifactStatus::Active,
            path: path.clone(),
            content_hash: new_hash.clone(),
            version: current_version,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
            pinned: false,
        });
        rec.version = updated_version;
        rec.content_hash = new_hash.clone();
        rec.updated_at_ms = now_ms();

        let store = match scope {
            LearningScope::Project => ctx.project_store,
            LearningScope::Global => ctx.global_store,
        };
        store
            .upsert_skill(rec)
            .map_err(|e| ToolError::Failed(format!("failed to update skill ledger: {}", e)))?;

        let body = json!({
            "status": "patched",
            "name": name,
            "version": updated_version,
            "contentHash": new_hash,
        });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }

    fn execute_write_file(
        ctx: SkillManagerContext<'_>,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let rel_path_str = args
            .get("filePath")
            .or_else(|| args.get("file"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("missing 'filePath'".into()))?;
        let validated_rel =
            validate_relative_support_path(rel_path_str).map_err(ToolError::Failed)?;

        if ctx.origin == SkillWriteOrigin::BackgroundReview
            && rel_path_str.replace('\\', "/").starts_with("scripts/")
        {
            return Err(ToolError::Failed(
                "background review cannot create scripts (fail closed)".into(),
            ));
        }

        // Locate skill directory
        let (skill_dir, scope, ledger_record) = if let Some(rec) = ctx.project_store.skill(name) {
            (
                rec.path.parent().unwrap_or(&rec.path).to_path_buf(),
                LearningScope::Project,
                Some(rec.clone()),
            )
        } else if let Some(rec) = ctx.global_store.skill(name) {
            (
                rec.path.parent().unwrap_or(&rec.path).to_path_buf(),
                LearningScope::Global,
                Some(rec.clone()),
            )
        } else if ctx.project_skills_dir.join(name).exists() {
            (
                ctx.project_skills_dir.join(name),
                LearningScope::Project,
                None,
            )
        } else if ctx.global_skills_dir.join(name).exists() {
            (
                ctx.global_skills_dir.join(name),
                LearningScope::Global,
                None,
            )
        } else {
            return Err(ToolError::Failed(format!("skill '{}' not found", name)));
        };

        let skill_origin = ledger_record
            .as_ref()
            .map(|r| r.origin)
            .unwrap_or(SkillOrigin::User);
        if ctx.origin == SkillWriteOrigin::BackgroundReview
            && (skill_origin == SkillOrigin::User || skill_origin == SkillOrigin::Imported)
        {
            return Err(ToolError::Failed(
                "background review cannot mutate user-origin or imported skills".into(),
            ));
        }

        if scope == LearningScope::Project
            && ctx.origin == SkillWriteOrigin::BackgroundReview
            && !ctx.project_trusted
        {
            return Err(ToolError::Failed(
                "untrusted project cannot receive autonomous writes".into(),
            ));
        }

        let target_file = skill_dir.join(validated_rel);

        // Check symlink escape
        if let Ok(canon_dir) = skill_dir.canonicalize() {
            if target_file.exists() {
                if let Ok(canon_target) = target_file.canonicalize() {
                    if !canon_target.starts_with(&canon_dir) {
                        return Err(ToolError::Failed("path escapes skill directory".into()));
                    }
                }
            } else if let Some(parent) = target_file.parent() {
                if parent.exists() {
                    if let Ok(canon_parent) = parent.canonicalize() {
                        if !canon_parent.starts_with(&canon_dir) {
                            return Err(ToolError::Failed(
                                "parent directory escapes skill directory via symlink".into(),
                            ));
                        }
                    }
                }
            }
        }

        let raw_content = args.get("content").and_then(Value::as_str).unwrap_or("");
        let content = redact_secrets(raw_content);

        atomic_write_file(&target_file, &content)
            .map_err(|e| ToolError::Failed(format!("failed to write support file: {}", e)))?;

        let hash_val = content_hash(&content);
        let body = json!({
            "status": "written",
            "name": name,
            "filePath": rel_path_str,
            "contentHash": hash_val,
        });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }

    fn execute_status_change(
        ctx: SkillManagerContext<'_>,
        name: &str,
        status: ArtifactStatus,
    ) -> Result<ToolResult, ToolError> {
        let record = if let Some(rec) = ctx.project_store.skill(name) {
            let mut r = rec.clone();
            r.status = status;
            ctx.project_store
                .upsert_skill(r.clone())
                .map_err(ToolError::Failed)?;
            r
        } else if let Some(rec) = ctx.global_store.skill(name) {
            let mut r = rec.clone();
            r.status = status;
            ctx.global_store
                .upsert_skill(r.clone())
                .map_err(ToolError::Failed)?;
            r
        } else {
            return Err(ToolError::Failed(format!(
                "skill '{}' not found in ledger",
                name
            )));
        };

        let body = json!({
            "status": format!("{:?}", status).to_lowercase(),
            "name": name,
            "scope": record.scope,
        });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&body).unwrap_or_default(),
            is_error: false,
            details: Some(body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_env() -> (
        tempfile::TempDir,
        PathBuf,
        PathBuf,
        LearningStore,
        LearningStore,
        ReviewReadSet,
    ) {
        let dir = tempdir().unwrap();
        let project_skills = dir.path().join("project_skills");
        let global_skills = dir.path().join("global_skills");
        let project_learning = dir.path().join("project_learning");
        let global_learning = dir.path().join("global_learning");
        fs::create_dir_all(&project_skills).unwrap();
        fs::create_dir_all(&global_skills).unwrap();
        let project_store = LearningStore::open(project_learning).unwrap();
        let global_store = LearningStore::open(global_learning).unwrap();
        let read_set = ReviewReadSet::new();
        (
            dir,
            project_skills,
            global_skills,
            project_store,
            global_store,
            read_set,
        )
    }

    #[test]
    fn background_review_creates_project_learned_skill_in_trusted_project_allowed() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, read_set) = setup_env();
        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };
        let args = json!({
            "action": "create",
            "name": "debug-sqlx",
            "scope": "project",
            "description": "Diagnose SQLx compile failures",
            "body": "## Instructions\nRun cargo check"
        });
        let res = SkillManager::execute(ctx, &args).unwrap();
        assert!(!res.is_error);
        assert!(p_skills.join("debug-sqlx").join("SKILL.md").exists());
        let rec = p_store.skill("debug-sqlx").unwrap();
        assert_eq!(rec.origin, SkillOrigin::LearnedReview);
        assert_eq!(rec.status, ArtifactStatus::Active);
    }

    #[test]
    fn background_review_creates_project_skill_in_untrusted_project_denied() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, read_set) = setup_env();
        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: false, // untrusted!
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };
        let args = json!({
            "action": "create",
            "name": "debug-sqlx",
            "scope": "project",
            "description": "Diagnose SQLx",
            "body": "Do something"
        });
        let err = SkillManager::execute(ctx, &args).unwrap_err();
        assert!(err.to_string().contains("untrusted project"));
    }

    #[test]
    fn background_review_patches_user_origin_skill_denied() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, read_set) = setup_env();
        let skill_dir = p_skills.join("user-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(&skill_file, "Original text").unwrap();

        p_store
            .upsert_skill(SkillLedgerRecord {
                skill_id: "user-skill".into(),
                name: "user-skill".into(),
                scope: LearningScope::Project,
                origin: SkillOrigin::User,
                status: ArtifactStatus::Active,
                path: skill_file.clone(),
                content_hash: content_hash("Original text"),
                version: 1,
                success_count: 0,
                failure_count: 0,
                neutral_count: 0,
                last_used_at_ms: None,
                created_at_ms: 1000,
                updated_at_ms: 1000,
                pinned: false,
            })
            .unwrap();

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };
        let args = json!({
            "action": "patch",
            "name": "user-skill",
            "oldText": "Original",
            "newText": "Patched",
            "expectedHash": content_hash("Original text")
        });
        let err = SkillManager::execute(ctx, &args).unwrap_err();
        assert!(err.to_string().contains("cannot mutate user-origin"));
    }

    #[test]
    fn background_review_patches_learned_skill_without_prior_skill_view_denied() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, read_set) = setup_env();
        let skill_dir = p_skills.join("learned-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(&skill_file, "Original text").unwrap();

        p_store
            .upsert_skill(SkillLedgerRecord {
                skill_id: "learned-skill".into(),
                name: "learned-skill".into(),
                scope: LearningScope::Project,
                origin: SkillOrigin::LearnedReview,
                status: ArtifactStatus::Active,
                path: skill_file.clone(),
                content_hash: content_hash("Original text"),
                version: 1,
                success_count: 0,
                failure_count: 0,
                neutral_count: 0,
                last_used_at_ms: None,
                created_at_ms: 1000,
                updated_at_ms: 1000,
                pinned: false,
            })
            .unwrap();

        // read_set is empty!
        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };
        let args = json!({
            "action": "patch",
            "name": "learned-skill",
            "oldText": "Original",
            "newText": "Patched",
            "expectedHash": content_hash("Original text")
        });
        let err = SkillManager::execute(ctx, &args).unwrap_err();
        assert!(err.to_string().contains("must skill_view"));
    }

    #[test]
    fn background_review_patches_learned_skill_with_stale_hash_denied() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, mut read_set) = setup_env();
        let skill_dir = p_skills.join("learned-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(&skill_file, "Original text modified").unwrap();

        read_set.record(skill_file.clone(), content_hash("Original text"));

        p_store
            .upsert_skill(SkillLedgerRecord {
                skill_id: "learned-skill".into(),
                name: "learned-skill".into(),
                scope: LearningScope::Project,
                origin: SkillOrigin::LearnedReview,
                status: ArtifactStatus::Active,
                path: skill_file.clone(),
                content_hash: content_hash("Original text modified"),
                version: 1,
                success_count: 0,
                failure_count: 0,
                neutral_count: 0,
                last_used_at_ms: None,
                created_at_ms: 1000,
                updated_at_ms: 1000,
                pinned: false,
            })
            .unwrap();

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };
        let args = json!({
            "action": "patch",
            "name": "learned-skill",
            "oldText": "Original",
            "newText": "Patched",
            "expectedHash": content_hash("Original text") // stale!
        });
        let err = SkillManager::execute(ctx, &args).unwrap_err();
        assert!(err.to_string().contains("skill changed since review"));
    }

    #[test]
    fn foreground_learn_patches_learned_skill_allowed() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, read_set) = setup_env();
        let skill_dir = p_skills.join("learned-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(&skill_file, "Original text").unwrap();

        p_store
            .upsert_skill(SkillLedgerRecord {
                skill_id: "learned-skill".into(),
                name: "learned-skill".into(),
                scope: LearningScope::Project,
                origin: SkillOrigin::LearnedReview,
                status: ArtifactStatus::Active,
                path: skill_file.clone(),
                content_hash: content_hash("Original text"),
                version: 1,
                success_count: 0,
                failure_count: 0,
                neutral_count: 0,
                last_used_at_ms: None,
                created_at_ms: 1000,
                updated_at_ms: 1000,
                pinned: false,
            })
            .unwrap();

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::ForegroundUserDirected,
            read_set: &read_set,
        };
        let args = json!({
            "action": "patch",
            "name": "learned-skill",
            "oldText": "Original text",
            "newText": "Patched text",
            "expectedHash": content_hash("Original text")
        });
        let res = SkillManager::execute(ctx, &args).unwrap();
        assert!(!res.is_error);
        assert_eq!(fs::read_to_string(&skill_file).unwrap(), "Patched text");
        assert_eq!(p_store.skill("learned-skill").unwrap().version, 2);
    }

    #[test]
    fn path_traversal_and_invalid_support_paths_denied() {
        assert!(validate_relative_support_path("../outside.md").is_err());
        assert!(validate_relative_support_path("references/../../escape.md").is_err());
        assert!(validate_relative_support_path("/absolute/path.md").is_err());
        assert!(validate_relative_support_path("assets/icon.png").is_err());
        assert!(validate_relative_support_path("references/doc.md").is_ok());
        assert!(validate_relative_support_path("templates/template.md").is_ok());
        assert!(validate_relative_support_path("scripts/run.sh").is_ok());
    }

    #[test]
    fn near_duplicate_skill_creation_denied() {
        let dir = tempfile::tempdir().unwrap();
        let p_skills = dir.path().join("p_skills");
        let g_skills = dir.path().join("g_skills");
        let mut p_store = LearningStore::open(dir.path().join("p_store")).unwrap();
        let mut g_store = LearningStore::open(dir.path().join("g_store")).unwrap();
        let read_set = ReviewReadSet::new();

        p_store
            .upsert_skill(SkillLedgerRecord {
                skill_id: "debug-sqlx".into(),
                name: "debug-sqlx".into(),
                scope: LearningScope::Project,
                origin: SkillOrigin::LearnedReview,
                status: ArtifactStatus::Active,
                path: p_skills.join("debug-sqlx").join("SKILL.md"),
                content_hash: "hash".into(),
                version: 1,
                success_count: 0,
                failure_count: 0,
                neutral_count: 0,
                last_used_at_ms: None,
                created_at_ms: 1000,
                updated_at_ms: 1000,
                pinned: false,
            })
            .unwrap();

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::ForegroundUserDirected,
            read_set: &read_set,
        };
        let args = json!({
            "action": "create",
            "name": "debug-sqlx",
            "body": "Duplicate skill body"
        });
        assert!(SkillManager::execute(ctx, &args).is_err());
    }

    #[test]
    fn rollback_history_sorts_numerically_and_keeps_latest_five() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, mut read_set) = setup_env();
        let skill_file = p_skills.join("learned-skill").join("SKILL.md");
        fs::create_dir_all(skill_file.parent().unwrap()).unwrap();
        fs::write(&skill_file, "Version 1 content").unwrap();

        let mut rec = SkillLedgerRecord {
            skill_id: "learned-skill".into(),
            name: "learned-skill".into(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: skill_file.clone(),
            content_hash: content_hash("Version 1 content"),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        };
        p_store.upsert_skill(rec.clone()).unwrap();

        // Simulate reaching version 9 with history files 1.md .. 8.md
        let history_dir = p_store.root().join("history").join("learned-skill");
        fs::create_dir_all(&history_dir).unwrap();
        for v in 1..=8 {
            fs::write(
                history_dir.join(format!("{}.md", v)),
                format!("Version {}", v),
            )
            .unwrap();
        }

        rec.version = 9;
        rec.content_hash = content_hash("Version 9 content");
        fs::write(&skill_file, "Version 9 content").unwrap();
        p_store.upsert_skill(rec).unwrap();

        read_set.record(skill_file.clone(), content_hash("Version 9 content"));

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };

        // Now patch version 9 -> version 10
        let args = json!({
            "action": "patch",
            "name": "learned-skill",
            "oldText": "Version 9",
            "newText": "Version 10",
            "expectedHash": content_hash("Version 9 content")
        });
        let res = SkillManager::execute(ctx, &args).unwrap();
        assert!(!res.is_error);

        // History directory must retain at most 5 files, and 9.md must be among them, not deleted!
        let mut entries: Vec<_> = fs::read_dir(&history_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        entries.sort();
        assert_eq!(entries.len(), 5);
        assert!(entries.contains(&"9.md".to_string()));
        assert!(!entries.contains(&"1.md".to_string()));
    }

    #[test]
    fn concurrent_external_modification_rejects_patch() {
        let (_dir, p_skills, g_skills, mut p_store, mut g_store, mut read_set) = setup_env();
        let skill_file = p_skills.join("learned-skill").join("SKILL.md");
        fs::create_dir_all(skill_file.parent().unwrap()).unwrap();
        fs::write(&skill_file, "Original text").unwrap();

        let rec = SkillLedgerRecord {
            skill_id: "learned-skill".into(),
            name: "learned-skill".into(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: skill_file.clone(),
            content_hash: content_hash("Original text"),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        };
        p_store.upsert_skill(rec).unwrap();
        read_set.record(skill_file.clone(), content_hash("Original text"));

        // External process modifies file right now
        fs::write(&skill_file, "Externally changed text").unwrap();

        let ctx = SkillManagerContext {
            project_skills_dir: &p_skills,
            global_skills_dir: &g_skills,
            project_store: &mut p_store,
            global_store: &mut g_store,
            project_trusted: true,
            auto_apply_global: false,
            origin: SkillWriteOrigin::BackgroundReview,
            read_set: &read_set,
        };

        let args = json!({
            "action": "patch",
            "name": "learned-skill",
            "oldText": "Original",
            "newText": "Patched",
            "expectedHash": content_hash("Original text")
        });
        let err = SkillManager::execute(ctx, &args).unwrap_err();
        assert!(err.to_string().contains("skill changed since review"));
    }
}
