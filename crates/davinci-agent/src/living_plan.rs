//! Session-owned implementation plans. Separate from the task-progress ledger.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const PLAN_ENTRY_TYPE: &str = "living_plan";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivingPlan {
    pub revision: u64,
    pub goal: String,
    pub evidence: Vec<Evidence>,
    pub assumptions: Vec<String>,
    pub questions: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub decisions: BTreeMap<String, bool>,
    pub approved_revision: Option<u64>,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    pub finding: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    pub id: String,
    pub change: String,
    pub files: Vec<String>,
    pub why: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub verify: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanUpdate {
    expected_revision: u64,
    goal: Option<String>,
    evidence: Option<Vec<EvidenceInput>>,
    assumptions: Option<Vec<String>>,
    questions: Option<Vec<String>>,
    #[serde(default)]
    steps: Vec<PlanStep>,
    #[serde(default)]
    remove_steps: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceInput {
    path: String,
    finding: String,
}

impl LivingPlan {
    /// Transactional partial updates: unspecified fields and steps are kept.
    /// The model cannot supply decisions or approval through this interface.
    pub fn update(&mut self, args: &Value, cwd: &Path) -> Result<(), String> {
        if args.to_string().len() > 65_536 {
            return Err("Plan update exceeds 64 KiB".into());
        }
        let update: PlanUpdate = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
        if update.expected_revision != self.revision {
            return Err(format!(
                "Plan revision conflict: expected {}, current {}. Read the current plan and retry.",
                update.expected_revision, self.revision
            ));
        }
        let mut next = self.clone();
        next.changes.clear();
        let mut context_changed = false;
        if let Some(goal) = update.goal {
            if next.goal != goal {
                next.goal = goal;
                context_changed = true;
                next.changes.push("goal revised".into());
            }
        }
        if let Some(assumptions) = update.assumptions {
            if next.assumptions != assumptions {
                next.assumptions = assumptions;
                context_changed = true;
                next.changes.push("assumptions revised".into());
            }
        }
        if let Some(questions) = update.questions {
            if next.questions != questions {
                next.questions = questions;
                context_changed = true;
                next.changes.push("open decisions revised".into());
            }
        }
        if let Some(inputs) = update.evidence {
            if inputs.len() > 32 {
                return Err("A plan may record at most 32 evidence files".into());
            }
            let mut evidence = Vec::new();
            let mut paths = std::collections::BTreeSet::new();
            for input in inputs {
                if !paths.insert(input.path.clone()) {
                    return Err(format!("Duplicate evidence path: {}", input.path));
                }
                nonempty(&input.finding, "evidence finding")?;
                let fingerprint = fingerprint(cwd, &input.path)?;
                evidence.push(Evidence {
                    path: input.path,
                    finding: input.finding,
                    fingerprint,
                });
            }
            if next.evidence != evidence {
                next.evidence = evidence;
                context_changed = true;
                next.changes.push("repository evidence refreshed".into());
            }
        }
        let mut changed = std::collections::BTreeSet::new();
        for id in update.remove_steps {
            let Some(index) = next.steps.iter().position(|s| s.id == id) else {
                return Err(format!("Unknown step: {id}"));
            };
            next.steps.remove(index);
            next.decisions.remove(&id);
            next.changes.push(format!("removed {id}"));
            changed.insert(id);
        }
        let mut supplied = std::collections::BTreeSet::new();
        for step in update.steps {
            if !supplied.insert(step.id.clone()) {
                return Err(format!("Duplicate step in update: {}", step.id));
            }
            if let Some(index) = next.steps.iter().position(|s| s.id == step.id) {
                if next.steps[index] == step {
                    continue;
                }
                next.changes.push(format!("revised {}", step.id));
                changed.insert(step.id.clone());
                next.steps[index] = step;
            } else {
                next.changes.push(format!("added {}", step.id));
                changed.insert(step.id.clone());
                next.steps.push(step);
            }
        }
        next.validate_structure()?;
        next.order_steps()?;
        if next.changes.is_empty() {
            return Ok(());
        }
        if context_changed {
            next.decisions.clear();
        } else {
            // A revised prerequisite invalidates its dependants, not unrelated work.
            loop {
                let before = changed.len();
                for step in &next.steps {
                    if step.depends_on.iter().any(|id| changed.contains(id)) {
                        changed.insert(step.id.clone());
                    }
                }
                if changed.len() == before {
                    break;
                }
            }
            next.decisions.retain(|id, _| !changed.contains(id));
        }
        next.bump_revision()?;
        if serde_json::to_vec(&next).map_err(|e| e.to_string())?.len() > 131_072 {
            return Err("Plan exceeds 128 KiB; keep evidence and changes concise".into());
        }
        *self = next;
        Ok(())
    }

    /// User-facing decision operation; deliberately absent from the model tool schema.
    pub fn decide(&mut self, id: &str, accept: bool) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err("There is no plan to review".into());
        }
        if id != "all" && !self.steps.iter().any(|s| s.id == id) {
            return Err(format!("Unknown step: {id}"));
        }
        self.bump_revision()?;
        for step in &self.steps {
            if id == "all" || step.id == id {
                self.decisions.insert(step.id.clone(), accept);
            }
        }
        self.changes = vec![format!(
            "{} {id}",
            if accept { "accepted" } else { "rejected" }
        )];
        Ok(())
    }

    pub fn edit_step(&mut self, id: &str, change: &str, cwd: &Path) -> Result<(), String> {
        let mut step = self
            .steps
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| format!("Unknown step: {id}"))?;
        step.change = change.to_string();
        self.update(
            &serde_json::json!({"expected_revision": self.revision, "steps": [step]}),
            cwd,
        )
    }

    /// Approves the current revision, not arbitrary future model output.
    pub fn approve(&mut self, cwd: &Path) -> Result<(), String> {
        self.validate_structure()?;
        if self.steps.is_empty() || self.evidence.is_empty() {
            return Err("An execution-ready plan needs steps and repository evidence".into());
        }
        if !self.questions.is_empty() {
            return Err("Resolve the plan's open decisions before approving".into());
        }
        if self.decisions.values().any(|accepted| !accepted) {
            return Err("Revise or explicitly accept rejected steps before approving".into());
        }
        self.check_fresh(cwd)?;
        for step in &self.steps {
            self.decisions.insert(step.id.clone(), true);
        }
        self.approved_revision = Some(self.revision);
        self.changes = vec![format!("approved revision {} for execution", self.revision)];
        Ok(())
    }

    pub fn ready(&self, cwd: &Path) -> Result<(), String> {
        if self.revision == 0 || self.approved_revision != Some(self.revision) {
            return Err(
                "Approve the current plan with /plan approve before handing it to execution".into(),
            );
        }
        self.validate_structure()?;
        if !self.questions.is_empty()
            || self.steps.is_empty()
            || self
                .steps
                .iter()
                .any(|s| self.decisions.get(&s.id) != Some(&true))
        {
            return Err("The plan contains unresolved or unapproved decisions".into());
        }
        self.check_fresh(cwd)
    }

    pub fn check_fresh(&self, cwd: &Path) -> Result<(), String> {
        for evidence in &self.evidence {
            match fingerprint(cwd, &evidence.path) {
                Ok(current) if current == evidence.fingerprint => {},
                _ => return Err(format!("Plan is stale: {} changed or is unavailable. Reinvestigate and revise its evidence.", evidence.path)),
            }
        }
        Ok(())
    }

    fn bump_revision(&mut self) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Plan revision counter exhausted")?;
        self.approved_revision = None;
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), String> {
        nonempty(&self.goal, "goal")?;
        if self.steps.len() > 64 || self.assumptions.len() > 32 || self.questions.len() > 32 {
            return Err("Keep plans within 64 steps, 32 assumptions and 32 open decisions".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for step in &self.steps {
            if step.id.is_empty()
                || step.id == "all"
                || step.id.len() > 48
                || !step
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err("Step IDs must be 1-48 letters, digits, hyphens or underscores; 'all' is reserved".into());
            }
            if !ids.insert(&step.id) {
                return Err(format!("Duplicate step: {}", step.id));
            }
            nonempty(&step.change, "step change")?;
            nonempty(&step.why, "step rationale")?;
            if step.files.is_empty()
                || step.verify.is_empty()
                || step.files.len() > 32
                || step.verify.len() > 16
            {
                return Err(format!(
                    "Step {} needs concrete files (1-32) and validation (1-16)",
                    step.id
                ));
            }
            for path in &step.files {
                relative_path(path)?;
            }
            for check in &step.verify {
                nonempty(check, "validation")?;
            }
        }
        for text in self.assumptions.iter().chain(&self.questions) {
            nonempty(text, "assumption or decision")?;
        }
        for step in &self.steps {
            for id in &step.depends_on {
                if !ids.contains(id) || id == &step.id {
                    return Err(format!("Invalid dependency {} -> {id}", step.id));
                }
            }
        }
        // Also validate the graph on restored session data.
        let mut copy = self.clone();
        copy.order_steps()
    }

    fn order_steps(&mut self) -> Result<(), String> {
        let mut pending = self.steps.clone();
        let mut ordered = Vec::with_capacity(pending.len());
        let mut done = std::collections::BTreeSet::new();
        while !pending.is_empty() {
            let Some(index) = pending
                .iter()
                .position(|s| s.depends_on.iter().all(|id| done.contains(id)))
            else {
                return Err("Plan dependencies contain a cycle or a missing step".into());
            };
            let step = pending.remove(index);
            done.insert(step.id.clone());
            ordered.push(step);
        }
        self.steps = ordered;
        Ok(())
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        if value.to_string().len() > 131_072 {
            return None;
        }
        let mut plan: Self = serde_json::from_value(value.clone()).ok()?;
        if plan.revision != 0 {
            plan.validate_structure().ok()?;
        }
        plan.approved_revision = None;
        Some(plan)
    }

    pub fn render(&self) -> String {
        if self.revision == 0 {
            return "No structured plan yet. Inspect the repository, then use propose_plan to record evidence, decisions, concrete steps and validation.".into();
        }
        let state = if self.approved_revision == Some(self.revision) {
            "approved"
        } else {
            "draft"
        };
        let mut lines = vec![format!(
            "# {}\nRevision {} · {state}",
            self.goal, self.revision
        )];
        if !self.changes.is_empty() {
            lines.push(format!("\nChanges: {}", self.changes.join("; ")));
        }
        lines.push("\n## Repository evidence".into());
        for e in &self.evidence {
            lines.push(format!(
                "- {}: {} [sha256 {}]",
                e.path,
                e.finding,
                e.fingerprint.get(..12).unwrap_or(&e.fingerprint)
            ));
        }
        if !self.assumptions.is_empty() {
            lines.push("\n## Assumptions".into());
            lines.extend(self.assumptions.iter().map(|s| format!("- {s}")));
        }
        if !self.questions.is_empty() {
            lines.push("\n## Open decisions (block approval)".into());
            lines.extend(self.questions.iter().map(|s| format!("- {s}")));
        }
        lines.push("\n## Implementation steps".into());
        for s in &self.steps {
            let decision = match self.decisions.get(&s.id) {
                Some(true) => "accepted",
                Some(false) => "rejected",
                None => "pending",
            };
            lines.push(format!(
                "\n### {} · {decision}\n{}\nFiles: {}\nWhy: {}\nDepends on: {}\nVerify: {}",
                s.id,
                s.change,
                s.files.join(", "),
                s.why,
                if s.depends_on.is_empty() {
                    "none".into()
                } else {
                    s.depends_on.join(", ")
                },
                s.verify.join("; ")
            ));
        }
        lines.push("\n/plan accept <id|all> · /plan reject <id> · /plan edit <id> <change> · /plan approve · /act".into());
        lines.join("\n")
    }
}

fn nonempty(text: &str, field: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > 8_192 {
        Err(format!(
            "{field} must contain 1-8192 bytes of meaningful text"
        ))
    } else {
        Ok(())
    }
}

/// Strict relative paths avoid device files, ADS and platform-dependent escapes.
fn relative_path(path: &str) -> Result<std::path::PathBuf, String> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.chars().any(char::is_control)
        || normalized.split('/').any(|part| {
            let base = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            part == ".."
                || part.is_empty()
                || (part != "." && part.ends_with(['.', ' ']))
                || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ((base.starts_with("COM") || base.starts_with("LPT"))
                    && base.len() == 4
                    && matches!(base.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err(format!(
            "Plan paths must stay relative to the workspace: {path}"
        ));
    }
    Ok(std::path::PathBuf::from(normalized))
}

/// Evidence never becomes an alternate file-reader with broader permissions.
/// No shell is executed; bounded reads are confined to ordinary workspace files.
fn fingerprint(cwd: &Path, path: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let relative = relative_path(path)?;
    if crate::permission::is_sensitive_file_path(path) {
        return Err(format!(
            "Sensitive files cannot be captured as plan evidence: {path}"
        ));
    }
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    if lower.split('/').any(|part| {
        matches!(
            part,
            ".git"
                | ".pi"
                | ".davinci"
                | ".claude"
                | ".codex"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".kube"
                | ".gnupg"
                | ".env"
                | ".envrc"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
                | ".git-credentials"
                | ".pgpass"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
                | "auth.json"
                | "service-account.json"
        ) || part.starts_with(".env.")
            || part.starts_with("id_rsa")
            || part.starts_with("id_ed25519")
            || [".pem", ".key", ".p12", ".pfx", ".keystore", ".kdbx", ".jks"]
                .iter()
                .any(|suffix| part.ends_with(suffix))
    }) {
        return Err(format!(
            "Sensitive files cannot be captured as plan evidence: {path}"
        ));
    }
    let root = cwd
        .canonicalize()
        .map_err(|e| format!("Cannot resolve workspace: {e}"))?;
    let file = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("Cannot capture evidence {path}: {e}"))?;
    if !file.starts_with(&root) {
        return Err(format!("Plan evidence escapes the workspace: {path}"));
    }
    let resolved = file
        .strip_prefix(&root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    if crate::permission::is_sensitive_file_path(&resolved)
        || resolved
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".claude") || part.eq_ignore_ascii_case(".codex"))
    {
        return Err(format!(
            "Plan evidence resolves to protected metadata: {path}"
        ));
    }
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    const MAX_BYTES: u64 = 1_048_576;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(format!(
            "Evidence must be a regular file of at most 1 MiB: {path}"
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&file)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("Evidence grew beyond 1 MiB: {path}"));
    }
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub fn tool_parameters() -> Value {
    serde_json::json!({
        "type":"object", "additionalProperties":false,
        "properties": {
            "expected_revision":{"type":"integer","minimum":0,"description":"Current revision, initially 0. A stale revision is refused."},
            "goal":{"type":"string"},
            "evidence":{"type":"array","maxItems":32,"items":{"type":"object","additionalProperties":false,
                "properties":{"path":{"type":"string"},"finding":{"type":"string"}},"required":["path","finding"]}},
            "assumptions":{"type":"array","items":{"type":"string"}},
            "questions":{"type":"array","items":{"type":"string"},"description":"Material unresolved decisions only. These block approval."},
            "steps":{"type":"array","maxItems":64,"description":"Upsert these stable IDs; unspecified steps and unaffected user decisions remain.",
                "items":{"type":"object","additionalProperties":false,"properties":{
                    "id":{"type":"string"},"change":{"type":"string"},"files":{"type":"array","items":{"type":"string"}},
                    "why":{"type":"string"},"depends_on":{"type":"array","items":{"type":"string"}},
                    "verify":{"type":"array","items":{"type":"string"}}},"required":["id","change","files","why","verify"]}},
            "remove_steps":{"type":"array","items":{"type":"string"}}
        }, "required":["expected_revision"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(id: &str) -> Value {
        json!({"id": id, "change": format!("Implement {id}"), "files": ["src.rs"],
            "why": "Required behavior", "depends_on": [], "verify": ["cargo test --offline"]})
    }

    fn fixture() -> (tempfile::TempDir, LivingPlan) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
        let mut plan = LivingPlan::default();
        plan.update(
            &json!({"expected_revision": 0, "goal": "Add a feature",
            "evidence": [{"path":"src.rs", "finding":"The existing entry point"}],
            "steps": [step("one"), step("two")]}),
            dir.path(),
        )
        .unwrap();
        (dir, plan)
    }

    #[test]
    fn plan_is_versioned_and_requires_explicit_human_approval() {
        let (dir, mut plan) = fixture();
        assert_eq!(plan.revision, 1);
        assert!(plan.ready(dir.path()).is_err());
        plan.approve(dir.path()).unwrap();
        plan.ready(dir.path()).unwrap();
        let restored: LivingPlan =
            serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(restored, plan);
    }

    #[test]
    fn revisions_preserve_unaffected_decisions_and_refuse_lost_updates() {
        let (dir, mut plan) = fixture();
        plan.decide("one", true).unwrap();
        plan.decide("two", true).unwrap();
        let revision = plan.revision;
        let mut changed = step("two");
        changed["change"] = json!("A revised implementation");
        plan.update(
            &json!({"expected_revision": revision, "steps": [changed]}),
            dir.path(),
        )
        .unwrap();
        assert_eq!(plan.decisions.get("one"), Some(&true));
        assert_eq!(plan.decisions.get("two"), None);
        let before = plan.clone();
        assert!(plan
            .update(
                &json!({"expected_revision":revision,"steps":[step("three")]}),
                dir.path()
            )
            .is_err());
        assert_eq!(before, plan);
    }

    #[test]
    fn changed_evidence_rejected_steps_and_open_questions_block_handoff() {
        let (dir, mut plan) = fixture();
        plan.decide("one", false).unwrap();
        assert!(plan.approve(dir.path()).is_err());
        plan.decide("one", true).unwrap();
        plan.approve(dir.path()).unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn changed() {}\n").unwrap();
        assert!(plan.ready(dir.path()).unwrap_err().contains("stale"));
    }

    #[test]
    fn invalid_dependencies_and_model_self_approval_are_rejected_atomically() {
        let (dir, mut plan) = fixture();
        let before = plan.clone();
        let mut bad = step("one");
        bad["depends_on"] = json!(["one"]);
        assert!(plan
            .update(
                &json!({"expected_revision":plan.revision,"steps":[bad]}),
                dir.path()
            )
            .is_err());
        assert_eq!(plan, before);
        assert!(plan
            .update(
                &json!({"expected_revision":plan.revision,"approved_revision":plan.revision}),
                dir.path()
            )
            .is_err());
        assert_eq!(plan, before);
    }

    #[test]
    fn evidence_rejects_sensitive_directories_and_platform_aliases() {
        let (dir, mut plan) = fixture();
        for path in [
            ".docker/config.json",
            "secrets/example.txt",
            "settings.xml",
            "gradle.properties",
        ] {
            let target = dir.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, "synthetic test fixture, not a secret").unwrap();
            assert!(
                plan.update(
                    &json!({"expected_revision":plan.revision,
                "evidence":[{"path":path,"finding":"fixture"}]}),
                    dir.path()
                )
                .is_err(),
                "{path}"
            );
        }
        for path in [
            ".env. ",
            "src.rs:stream",
            "folder. /src.rs",
            "NUL",
            "aux.txt",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn open_questions_and_dependency_changes_invalidate_human_approval() {
        let (dir, mut plan) = fixture();
        let mut two = step("two");
        two["depends_on"] = json!(["one"]);
        plan.update(
            &json!({"expected_revision":plan.revision,"steps":[two]}),
            dir.path(),
        )
        .unwrap();
        plan.approve(dir.path()).unwrap();
        plan.edit_step("one", "Revised prerequisite", dir.path())
            .unwrap();
        assert!(plan.decisions.is_empty());
        plan.update(
            &json!({"expected_revision":plan.revision,"questions":["Choose an external service"]}),
            dir.path(),
        )
        .unwrap();
        assert!(plan.approve(dir.path()).is_err());
        let before = plan.clone();
        plan.update(&json!({"expected_revision":plan.revision}), dir.path())
            .unwrap();
        assert_eq!(
            before, plan,
            "no-op updates preserve the revision and decisions"
        );
    }

    #[test]
    fn evidence_cannot_escape_workspace_or_read_sensitive_files() {
        let (dir, mut plan) = fixture();
        for path in [
            "../outside.txt",
            ".env",
            ".git/config",
            ".davinci/settings.json",
            "credentials.json",
        ] {
            let before = plan.clone();
            assert!(
                plan.update(
                    &json!({"expected_revision":plan.revision,
                "evidence":[{"path":path,"finding":"test"}]}),
                    dir.path()
                )
                .is_err(),
                "{path}"
            );
            assert_eq!(before, plan);
        }
    }
}
