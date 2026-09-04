use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningCandidate, SkillLedgerRecord, SkillOutcome, SkillVersionRef,
};

#[derive(Debug, Clone)]
pub struct LearningStore {
    root: PathBuf,
    candidates: BTreeMap<String, LearningCandidate>,
    skills: BTreeMap<String, SkillLedgerRecord>,
    skill_versions: BTreeMap<(String, u64), SkillLedgerRecord>,
    #[allow(dead_code)]
    diagnostics: Vec<String>,
}

impl LearningStore {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if !root.exists() {
            fs::create_dir_all(&root)
                .map_err(|e| format!("failed to create dir {:?}: {}", root, e))?;
        }

        let mut candidates = BTreeMap::new();
        let mut skills: BTreeMap<String, SkillLedgerRecord> = BTreeMap::new();
        let mut skill_versions = BTreeMap::new();
        let mut diagnostics = Vec::new();

        let candidates_path = root.join("candidates.jsonl");
        if candidates_path.exists() {
            if let Ok(content) = fs::read_to_string(&candidates_path) {
                for (line_no, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<LearningCandidate>(trimmed) {
                        Ok(candidate) => {
                            candidates.insert(candidate.id.clone(), candidate);
                        }
                        Err(err) => {
                            diagnostics.push(format!(
                                "candidates.jsonl line {}: malformed record: {}",
                                line_no + 1,
                                err
                            ));
                        }
                    }
                }
            }
        }

        let skills_path = root.join("skills.jsonl");
        if skills_path.exists() {
            if let Ok(content) = fs::read_to_string(&skills_path) {
                for (line_no, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<SkillLedgerRecord>(trimmed) {
                        Ok(skill) => {
                            skill_versions
                                .insert((skill.name.clone(), skill.version as u64), skill.clone());
                            if let Some(existing) = skills.get(&skill.name) {
                                if skill.version >= existing.version {
                                    skills.insert(skill.name.clone(), skill);
                                }
                            } else {
                                skills.insert(skill.name.clone(), skill);
                            }
                        }
                        Err(err) => {
                            diagnostics.push(format!(
                                "skills.jsonl line {}: malformed record: {}",
                                line_no + 1,
                                err
                            ));
                        }
                    }
                }
            }
        }

        Ok(Self {
            root,
            candidates,
            skills,
            skill_versions,
            diagnostics,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn candidate(&self, id: &str) -> Option<&LearningCandidate> {
        self.candidates.get(id)
    }

    pub fn candidates(&self) -> Vec<LearningCandidate> {
        self.candidates.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn candidate_refs(&self) -> Vec<&LearningCandidate> {
        self.candidates.values().collect()
    }

    pub fn upsert_candidate(&mut self, candidate: LearningCandidate) -> Result<(), String> {
        self.candidates
            .insert(candidate.id.clone(), candidate.clone());
        let candidates_path = self.root.join("candidates.jsonl");
        let line = serde_json::to_string(&candidate).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&candidates_path)
            .map_err(|e| format!("failed to open {:?}: {}", candidates_path, e))?;
        writeln!(file, "{}", line)
            .map_err(|e| format!("failed to append to {:?}: {}", candidates_path, e))?;
        let _ = self.save_state();
        Ok(())
    }

    #[allow(dead_code)]
    pub fn set_candidate_status(
        &mut self,
        id: &str,
        status: ArtifactStatus,
    ) -> Result<LearningCandidate, String> {
        let mut candidate = self
            .candidates
            .get(id)
            .cloned()
            .ok_or_else(|| format!("candidate not found: {}", id))?;
        candidate.status = status;
        self.upsert_candidate(candidate.clone())?;
        Ok(candidate)
    }

    pub fn skill(&self, name: &str) -> Option<&SkillLedgerRecord> {
        self.skills.get(name)
    }

    pub fn skills(&self) -> Vec<SkillLedgerRecord> {
        self.skills.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn skill_refs(&self) -> Vec<&SkillLedgerRecord> {
        self.skills.values().collect()
    }

    pub fn upsert_skill(&mut self, skill: SkillLedgerRecord) -> Result<(), String> {
        self.skill_versions
            .insert((skill.name.clone(), skill.version as u64), skill.clone());
        if let Some(existing) = self.skills.get(&skill.name) {
            if skill.version >= existing.version {
                self.skills.insert(skill.name.clone(), skill.clone());
            }
        } else {
            self.skills.insert(skill.name.clone(), skill.clone());
        }
        let skills_path = self.root.join("skills.jsonl");
        let line = serde_json::to_string(&skill).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&skills_path)
            .map_err(|e| format!("failed to open {:?}: {}", skills_path, e))?;
        writeln!(file, "{}", line)
            .map_err(|e| format!("failed to append to {:?}: {}", skills_path, e))?;
        let _ = self.save_state();
        Ok(())
    }

    pub fn skill_version(&self, name: &str, version: u64) -> Option<&SkillLedgerRecord> {
        self.skill_versions.get(&(name.to_string(), version))
    }

    pub fn record_skill_outcome(
        &mut self,
        name: &str,
        outcome: SkillOutcome,
    ) -> Result<bool, String> {
        if let Some(mut record) = self.skills.get(name).cloned() {
            match outcome {
                SkillOutcome::VerifiedSuccess => {
                    record.success_count += 1;
                }
                SkillOutcome::VerifiedFailure => {
                    record.failure_count += 1;
                }
                SkillOutcome::Neutral => {
                    record.neutral_count += 1;
                }
            }
            let now = crate::native_extensions::learning::types::now_ms();
            record.last_used_at_ms = Some(now);
            record.updated_at_ms = now;
            self.upsert_skill(record)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn record_skill_version_outcome(
        &mut self,
        skill: &SkillVersionRef,
        outcome: SkillOutcome,
    ) -> Result<(), String> {
        let key = (skill.name.clone(), skill.version);
        if let Some(mut record) = self.skill_versions.get(&key).cloned() {
            if record.content_hash != skill.content_hash {
                self.diagnostics.push(format!(
                    "skill version content hash mismatch for {}: expected {}, found {}",
                    skill.name, skill.content_hash, record.content_hash
                ));
                return Ok(());
            }
            match outcome {
                SkillOutcome::VerifiedSuccess => {
                    record.success_count += 1;
                }
                SkillOutcome::VerifiedFailure => {
                    record.failure_count += 1;
                }
                SkillOutcome::Neutral => {
                    record.neutral_count += 1;
                }
            }
            let now = crate::native_extensions::learning::types::now_ms();
            record.last_used_at_ms = Some(now);
            record.updated_at_ms = now;
            self.upsert_skill(record)?;
            Ok(())
        } else {
            self.diagnostics.push(format!(
                "skill version not found for outcome: {} v{}",
                skill.name, skill.version
            ));
            Ok(())
        }
    }

    pub fn save_state(&self) -> Result<(), String> {
        let state = crate::native_extensions::learning::types::LearningStoreState {
            last_updated_ms: crate::native_extensions::learning::types::now_ms(),
            candidate_count: self.candidates.len(),
            skill_count: self.skills.len(),
            version: 1,
        };
        let state_path = self.root.join("state.json");
        let tmp_path = self.root.join(format!(
            "state.json.tmp.{}",
            crate::native_extensions::learning::types::now_ms()
        ));
        let json_str = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        fs::write(&tmp_path, json_str).map_err(|e| e.to_string())?;
        if state_path.exists() {
            let _ = fs::remove_file(&state_path);
        }
        fs::rename(&tmp_path, &state_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn compact(&mut self) -> Result<(), String> {
        let now = crate::native_extensions::learning::types::now_ms();
        let candidates_path = self.root.join("candidates.jsonl");
        let candidates_tmp = self.root.join(format!("candidates.jsonl.tmp.{}", now));
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&candidates_tmp)
                .map_err(|e| format!("failed to open {:?}: {}", candidates_tmp, e))?;
            for cand in self.candidates.values() {
                let line = serde_json::to_string(cand).map_err(|e| e.to_string())?;
                writeln!(file, "{}", line).map_err(|e| format!("failed to write line: {}", e))?;
            }
        }
        if candidates_path.exists() {
            let _ = fs::remove_file(&candidates_path);
        }
        fs::rename(&candidates_tmp, &candidates_path).map_err(|e| e.to_string())?;

        let skills_path = self.root.join("skills.jsonl");
        let skills_tmp = self.root.join(format!("skills.jsonl.tmp.{}", now));
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&skills_tmp)
                .map_err(|e| format!("failed to open {:?}: {}", skills_tmp, e))?;
            for sk in self.skill_versions.values() {
                let line = serde_json::to_string(sk).map_err(|e| e.to_string())?;
                writeln!(file, "{}", line).map_err(|e| format!("failed to write line: {}", e))?;
            }
        }
        if skills_path.exists() {
            let _ = fs::remove_file(&skills_path);
        }
        fs::rename(&skills_tmp, &skills_path).map_err(|e| e.to_string())?;

        self.save_state()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn reload(&mut self) -> Result<(), String> {
        let candidates_path = self.root.join("candidates.jsonl");
        if candidates_path.exists() {
            if let Ok(content) = fs::read_to_string(&candidates_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(candidate) = serde_json::from_str::<LearningCandidate>(trimmed) {
                        self.candidates.insert(candidate.id.clone(), candidate);
                    }
                }
            }
        }

        let skills_path = self.root.join("skills.jsonl");
        if skills_path.exists() {
            if let Ok(content) = fs::read_to_string(&skills_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(skill) = serde_json::from_str::<SkillLedgerRecord>(trimmed) {
                        self.skill_versions
                            .insert((skill.name.clone(), skill.version as u64), skill.clone());
                        if let Some(existing) = self.skills.get(&skill.name) {
                            if skill.version >= existing.version {
                                self.skills.insert(skill.name.clone(), skill);
                            }
                        } else {
                            self.skills.insert(skill.name.clone(), skill);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::{
        LearningArtifact, LearningScope, VerificationEvidence,
    };

    fn fixture_candidate(id: &str) -> LearningCandidate {
        LearningCandidate {
            id: id.to_string(),
            scope: LearningScope::Project,
            status: ArtifactStatus::Candidate,
            artifact: LearningArtifact::SkillCreate {
                name: "test-skill".to_string(),
                description: "A test skill".to_string(),
                body: "Body content".to_string(),
            },
            confidence: 0.9,
            source_session_id: "sess-1".to_string(),
            source_repo_id: "repo-1".to_string(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence: VerificationEvidence::default(),
            rationale: "Good pattern".to_string(),
        }
    }

    #[test]
    fn candidate_round_trips_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let candidate = fixture_candidate("cand-1");
        store.upsert_candidate(candidate.clone()).unwrap();
        drop(store);

        let store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(store.candidate("cand-1"), Some(&candidate));
    }

    #[test]
    fn malformed_jsonl_line_does_not_destroy_valid_records() {
        let dir = tempfile::tempdir().unwrap();
        let candidates_path = dir.path().join("candidates.jsonl");
        let valid = fixture_candidate("cand-survivor");
        let valid_json = serde_json::to_string(&valid).unwrap();
        fs::write(&candidates_path, format!("{}\n{{broken\n", valid_json)).unwrap();

        let store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(store.candidate("cand-survivor"), Some(&valid));
        assert_eq!(store.diagnostics().len(), 1);
        assert!(store.diagnostics()[0].contains("line 2"));
    }

    #[test]
    fn skill_outcome_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let record = SkillLedgerRecord {
            skill_id: "skill-debug-sqlx".into(),
            name: "debug-sqlx".into(),
            scope: LearningScope::Project,
            origin: crate::native_extensions::learning::types::SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: dir.path().join("SKILL.md"),
            content_hash: "hash".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        };
        store.upsert_skill(record).unwrap();

        assert!(store
            .record_skill_outcome("debug-sqlx", SkillOutcome::VerifiedSuccess)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 0);
        assert_eq!(updated.neutral_count, 0);

        assert!(store
            .record_skill_outcome("debug-sqlx", SkillOutcome::VerifiedFailure)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.neutral_count, 0);

        assert!(store
            .record_skill_outcome("debug-sqlx", SkillOutcome::Neutral)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.neutral_count, 1);

        assert!(!store
            .record_skill_outcome("non-existent", SkillOutcome::Neutral)
            .unwrap());
    }

    #[test]
    fn state_json_written_on_upsert_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let candidate = fixture_candidate("cand-state");
        store.upsert_candidate(candidate).unwrap();

        let state_path = dir.path().join("state.json");
        assert!(state_path.exists());
        let raw = fs::read_to_string(&state_path).unwrap();
        let state: crate::native_extensions::learning::types::LearningStoreState =
            serde_json::from_str(&raw).unwrap();
        assert_eq!(state.candidate_count, 1);
        assert_eq!(state.skill_count, 0);
    }

    #[test]
    fn store_compaction_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let mut c1 = fixture_candidate("cand-1");
        c1.confidence = 0.6;
        store.upsert_candidate(c1.clone()).unwrap();
        // Update candidate (appends second record in jsonl)
        c1.confidence = 0.95;
        store.upsert_candidate(c1.clone()).unwrap();

        let content_before = fs::read_to_string(dir.path().join("candidates.jsonl")).unwrap();
        assert_eq!(content_before.lines().count(), 2);

        // Compact
        store.compact().unwrap();
        let content_after = fs::read_to_string(dir.path().join("candidates.jsonl")).unwrap();
        assert_eq!(content_after.lines().count(), 1);

        // Reload into a second instance
        let mut store2 = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(store2.candidate("cand-1").unwrap().confidence, 0.95);

        // External update simulated
        let c2 = fixture_candidate("cand-2");
        store.upsert_candidate(c2).unwrap();
        store2.reload().unwrap();
        assert!(store2.candidate("cand-2").is_some());
    }

    #[test]
    fn skill_version_outcome_attribution_only_affects_targeted_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let v1 = SkillLedgerRecord {
            skill_id: "skill-refactor".into(),
            name: "refactor".into(),
            scope: LearningScope::Project,
            origin: crate::native_extensions::learning::types::SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: dir.path().join("SKILL.md"),
            content_hash: "hash-v1".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        };
        let v2 = SkillLedgerRecord {
            skill_id: "skill-refactor".into(),
            name: "refactor".into(),
            scope: LearningScope::Project,
            origin: crate::native_extensions::learning::types::SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: dir.path().join("SKILL.md"),
            content_hash: "hash-v2".into(),
            version: 2,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 2000,
            updated_at_ms: 2000,
            pinned: false,
        };
        store.upsert_skill(v1).unwrap();
        store.upsert_skill(v2).unwrap();

        let v1_ref = SkillVersionRef {
            name: "refactor".into(),
            version: 1,
            content_hash: "hash-v1".into(),
        };

        store
            .record_skill_version_outcome(&v1_ref, SkillOutcome::VerifiedSuccess)
            .unwrap();

        let rec_v1 = store.skill_version("refactor", 1).unwrap();
        assert_eq!(rec_v1.success_count, 1);
        assert_eq!(rec_v1.failure_count, 0);

        let rec_v2 = store.skill_version("refactor", 2).unwrap();
        assert_eq!(rec_v2.success_count, 0);
        assert_eq!(rec_v2.failure_count, 0);

        // Hash mismatch records diagnostic and does not attribute to newer or older version
        let mismatched_ref = SkillVersionRef {
            name: "refactor".into(),
            version: 2,
            content_hash: "wrong-hash".into(),
        };
        store
            .record_skill_version_outcome(&mismatched_ref, SkillOutcome::VerifiedSuccess)
            .unwrap();
        let rec_v2_after = store.skill_version("refactor", 2).unwrap();
        assert_eq!(rec_v2_after.success_count, 0);
        assert!(store
            .diagnostics()
            .iter()
            .any(|d| d.contains("content hash mismatch")));

        // Non-existent version
        let missing_ref = SkillVersionRef {
            name: "refactor".into(),
            version: 99,
            content_hash: "hash-99".into(),
        };
        store
            .record_skill_version_outcome(&missing_ref, SkillOutcome::VerifiedSuccess)
            .unwrap();
        assert!(store
            .diagnostics()
            .iter()
            .any(|d| d.contains("not found for outcome")));
    }
}
