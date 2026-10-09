use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::native_extensions::learning::types::{
    ArtifactStatus, LearningCandidate, SkillLedgerRecord, SkillOutcome, SkillVersionRef,
};

/// Reads one JSONL journal. A missing file is `None`. Read errors and invalid
/// UTF-8 inside a completed record are errors: opening as an empty store would
/// let the next write or compaction overwrite the learned history. Invalid
/// bytes in the unterminated final record are a crash-torn append; they are
/// left out here and removed by `repair_journal_tail` before the next append.
fn read_journal(path: &Path) -> Result<Option<String>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to read {path:?}: {err}")),
    };
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(err) => {
            let valid_up_to = err.utf8_error().valid_up_to();
            let bytes = err.into_bytes();
            match bytes.iter().rposition(|byte| *byte == b'\n') {
                Some(newline) if valid_up_to > newline => {
                    String::from_utf8(bytes[..=newline].to_vec())
                        .map(Some)
                        .map_err(|err| format!("{path:?} is not valid UTF-8: {err}"))
                }
                _ => Err(format!(
                    "{path:?} is not valid UTF-8 (first bad byte at offset {valid_up_to}); \
                     refusing to open it as an empty store"
                )),
            }
        }
    }
}

/// Makes the journal end on a record boundary before appending. A complete
/// final record that only lacks its newline is terminated; a torn one is saved
/// byte-for-byte next to the journal and cut off, so a new record is never
/// glued onto it. Returns a diagnostic when it had to cut.
fn repair_journal_tail(path: &Path) -> Result<Option<String>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to inspect {path:?}: {err}")),
    };
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(None);
    }
    let tail_start = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let tail = &bytes[tail_start..];
    let complete = std::str::from_utf8(tail)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok())
        .is_some_and(|value| value.is_object());
    if complete {
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("failed to open {path:?}: {e}"))?;
        file.write_all(b"\n")
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("failed to terminate {path:?}: {e}"))?;
        return Ok(None);
    }
    let mut backup = path.as_os_str().to_owned();
    backup.push(format!(".torn-{}.bak", crate::native_extensions::learning::types::now_ms()));
    let backup = PathBuf::from(backup);
    fs::write(&backup, tail).map_err(|e| format!("failed to preserve torn tail {backup:?}: {e}"))?;
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| format!("failed to open {path:?}: {e}"))?;
    file.set_len(tail_start as u64)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("failed to cut torn tail of {path:?}: {e}"))?;
    Ok(Some(format!(
        "{path:?}: cut a torn final record ({} bytes) kept in {backup:?}",
        tail.len()
    )))
}

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
            fs::create_dir_all(&root).map_err(|e| format!("failed to create dir {root:?}: {e}"))?;
        }

        let mut candidates = BTreeMap::new();
        let mut skills: BTreeMap<String, SkillLedgerRecord> = BTreeMap::new();
        let mut skill_versions = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let mut candidate_line_count = 0usize;
        let mut skill_line_count = 0usize;

        let candidates_path = root.join("candidates.jsonl");
        if candidates_path.exists() {
            if let Some(content) = read_journal(&candidates_path)? {
                candidate_line_count = content.lines().count();
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
            if let Some(content) = read_journal(&skills_path)? {
                skill_line_count = content.lines().count();
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

        let mut store = Self {
            root,
            candidates,
            skills,
            skill_versions,
            diagnostics,
        };
        let candidates_bloated = candidate_line_count > 2 * store.candidates.len() + 32;
        let skills_bloated = skill_line_count > 2 * store.skill_versions.len() + 32;
        if candidates_bloated || skills_bloated {
            store.compact()?;
        }
        Ok(store)
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
        // Durable first: a failed append must leave the in-memory view as it was.
        let line = serde_json::to_string(&candidate).map_err(|e| e.to_string())?;
        let candidates_path = self.root.join("candidates.jsonl");
        self.append_line(&candidates_path, &line)?;
        self.candidates.insert(candidate.id.clone(), candidate);
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
            .ok_or_else(|| format!("candidate not found: {id}"))?;
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
        // Durable first: a failed append must leave the in-memory view as it was.
        let line = serde_json::to_string(&skill).map_err(|e| e.to_string())?;
        let skills_path = self.root.join("skills.jsonl");
        self.append_line(&skills_path, &line)?;
        self.skill_versions
            .insert((skill.name.clone(), skill.version as u64), skill.clone());
        if self
            .skills
            .get(&skill.name)
            .is_none_or(|existing| skill.version >= existing.version)
        {
            self.skills.insert(skill.name.clone(), skill);
        }
        let _ = self.save_state();
        Ok(())
    }

    /// Repair a torn tail left by a crash, then append durably.
    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), String> {
        if let Some(diagnostic) = repair_journal_tail(path)? {
            self.diagnostics.push(diagnostic);
        }
        append_record(path, line)
    }

    #[allow(dead_code)]
    pub fn skill_version(&self, name: &str, version: u64) -> Option<&SkillLedgerRecord> {
        self.skill_versions.get(&(name.to_string(), version))
    }

    pub fn skill_version_ref_for_content_hash(
        &self,
        name: &str,
        content_hash: &str,
    ) -> Option<SkillVersionRef> {
        self.skill_versions
            .values()
            .filter(|record| record.name == name && record.content_hash == content_hash)
            .max_by_key(|record| record.version)
            .map(|record| SkillVersionRef {
                name: record.name.clone(),
                version: record.version as u64,
                content_hash: record.content_hash.clone(),
            })
    }

    pub fn record_skill_version_outcome(
        &mut self,
        skill: &SkillVersionRef,
        outcome: SkillOutcome,
    ) -> Result<bool, String> {
        let key = (skill.name.clone(), skill.version);
        if let Some(mut record) = self.skill_versions.get(&key).cloned() {
            if record.content_hash != skill.content_hash {
                self.diagnostics.push(format!(
                    "skill version content hash mismatch for {}: expected {}, found {}",
                    skill.name, skill.content_hash, record.content_hash
                ));
                return Ok(false);
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
            Ok(true)
        } else {
            self.diagnostics.push(format!(
                "skill version not found for outcome: {} v{}",
                skill.name, skill.version
            ));
            Ok(false)
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
        let json_str = serde_json::to_string_pretty(&state).map_err(|err| err.to_string())?;
        davinci_sys::fs::atomic_write(&state_path, json_str.as_bytes())
            .map_err(|err| err.to_string())
    }

    pub fn compact(&mut self) -> Result<(), String> {
        let candidates_path = self.root.join("candidates.jsonl");
        let mut candidates = String::new();
        for candidate in self.candidates.values() {
            candidates.push_str(&serde_json::to_string(candidate).map_err(|err| err.to_string())?);
            candidates.push('\n');
        }
        davinci_sys::fs::atomic_write(&candidates_path, candidates.as_bytes())
            .map_err(|err| err.to_string())?;

        let skills_path = self.root.join("skills.jsonl");
        let mut skills = String::new();
        for skill in self.skill_versions.values() {
            skills.push_str(&serde_json::to_string(skill).map_err(|err| err.to_string())?);
            skills.push('\n');
        }
        davinci_sys::fs::atomic_write(&skills_path, skills.as_bytes())
            .map_err(|err| err.to_string())?;

        self.save_state()
    }

    #[allow(dead_code)]
    pub fn reload(&mut self) -> Result<(), String> {
        let candidates_path = self.root.join("candidates.jsonl");
        if candidates_path.exists() {
            if let Some(content) = read_journal(&candidates_path)? {
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
            if let Some(content) = read_journal(&skills_path)? {
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

    #[allow(dead_code)]
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

/// Append one JSONL record and flush it to disk before the caller publishes
/// the change in memory.
fn append_record(path: &Path, line: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("failed to open {path:?}: {e}"))?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_data())
        .map_err(|e| format!("failed to append to {path:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::learning::types::{
        LearningArtifact, LearningScope, VerificationEvidence,
    };

    #[test]
    fn failed_append_leaves_in_memory_state_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let original = fixture_candidate("cand-1");
        store.upsert_candidate(original.clone()).unwrap();
        // A directory where the ledger file should be makes every append fail.
        let candidates = dir.path().join("candidates.jsonl");
        fs::remove_file(&candidates).unwrap();
        fs::create_dir(&candidates).unwrap();

        let mut changed = original.clone();
        changed.rationale = "not durable".into();
        assert!(store.upsert_candidate(changed).is_err());
        assert!(store.upsert_candidate(fixture_candidate("cand-2")).is_err());
        assert_eq!(store.candidate("cand-1"), Some(&original));
        assert!(store.candidate("cand-2").is_none());
    }

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
    fn append_after_a_torn_final_record_is_not_glued_onto_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("candidates.jsonl");
        let first = serde_json::to_string(&fixture_candidate("cand-first")).unwrap();
        fs::write(&path, format!("{first}\n{{\"id\":")).unwrap();

        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        store.upsert_candidate(fixture_candidate("cand-second")).unwrap();
        assert!(store.diagnostics().iter().any(|d| d.contains("torn")));

        let reopened = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert!(reopened.candidate("cand-first").is_some());
        assert!(reopened.candidate("cand-second").is_some());
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".torn-"))
            .collect();
        assert_eq!(fs::read(backups[0].path()).unwrap(), b"{\"id\":");
    }

    #[test]
    fn complete_final_record_missing_its_newline_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("candidates.jsonl");
        let first = serde_json::to_string(&fixture_candidate("cand-first")).unwrap();
        fs::write(&path, &first).unwrap();

        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        store.upsert_candidate(fixture_candidate("cand-second")).unwrap();
        let reopened = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert!(reopened.candidate("cand-first").is_some());
        assert!(reopened.candidate("cand-second").is_some());
    }

    #[test]
    fn invalid_utf8_in_a_completed_record_fails_open_and_keeps_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("candidates.jsonl");
        let first = serde_json::to_string(&fixture_candidate("cand-first")).unwrap();
        let mut bytes = format!("{first}\n").into_bytes();
        bytes.extend_from_slice(b"{\"id\":\"\xFF\"}\n");
        fs::write(&path, &bytes).unwrap();

        let err = LearningStore::open(dir.path().to_path_buf()).unwrap_err();
        assert!(err.contains("not valid UTF-8"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn partial_utf8_tail_is_a_torn_append_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("candidates.jsonl");
        let first = serde_json::to_string(&fixture_candidate("cand-first")).unwrap();
        let mut bytes = format!("{first}\n{{\"id\":\"").into_bytes();
        bytes.extend_from_slice(&[0xF0, 0x9F]);
        fs::write(&path, &bytes).unwrap();

        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert!(store.candidate("cand-first").is_some());
        store.upsert_candidate(fixture_candidate("cand-second")).unwrap();
        let reopened = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert!(reopened.candidate("cand-second").is_some());
    }

    #[test]
    fn malformed_jsonl_line_does_not_destroy_valid_records() {
        let dir = tempfile::tempdir().unwrap();
        let candidates_path = dir.path().join("candidates.jsonl");
        let valid = fixture_candidate("cand-survivor");
        let valid_json = serde_json::to_string(&valid).unwrap();
        fs::write(&candidates_path, format!("{valid_json}\n{{broken\n")).unwrap();

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
            applicability: Default::default(),
            pinned: false,
        };
        store.upsert_skill(record).unwrap();

        let skill_ref = SkillVersionRef {
            name: "debug-sqlx".into(),
            version: 1,
            content_hash: "hash".into(),
        };
        assert!(store
            .record_skill_version_outcome(&skill_ref, SkillOutcome::VerifiedSuccess)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 0);
        assert_eq!(updated.neutral_count, 0);

        assert!(store
            .record_skill_version_outcome(&skill_ref, SkillOutcome::VerifiedFailure)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.neutral_count, 0);

        assert!(store
            .record_skill_version_outcome(&skill_ref, SkillOutcome::Neutral)
            .unwrap());
        let updated = store.skill("debug-sqlx").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.neutral_count, 1);

        assert!(!store
            .record_skill_version_outcome(
                &SkillVersionRef {
                    name: "non-existent".into(),
                    version: 1,
                    content_hash: "missing".into(),
                },
                SkillOutcome::Neutral,
            )
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
    fn opening_a_bloated_ledger_compacts_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
        let mut candidate = fixture_candidate("cand-bloated");
        for i in 0..100 {
            candidate.confidence = (i as f32) / 100.0;
            store.upsert_candidate(candidate.clone()).unwrap();
        }
        drop(store);

        let path = dir.path().join("candidates.jsonl");
        assert!(fs::read_to_string(&path).unwrap().lines().count() >= 100);
        let reopened = LearningStore::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(reopened.candidate("cand-bloated").unwrap().confidence, 0.99);
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
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
            applicability: Default::default(),
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
            applicability: Default::default(),
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
