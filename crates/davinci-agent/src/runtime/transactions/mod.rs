//! Durable lifecycle around the existing patch/effect contracts.
//! Authorization is supplied by the current host; records never grant authority.
mod api;
mod commit;
mod files;
mod macos_acl;
mod model;
mod store;
mod unix_xattrs;
mod verification;
pub use verification::{TransactionVerification, VerificationObservation};
pub(crate) mod tools;
#[cfg(windows)]
mod windows_acl;
#[cfg(windows)]
mod windows_streams;
pub(crate) use api::execute;
pub use api::{is_tool, tool_specs};
use model::*;
pub use model::{
    ProposedChange, SourceSnapshot, TransactionOwner, TransactionState, TransactionSummary,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use store::Store;
pub use tools::MutationAuthority;

type Authority<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

#[derive(Debug, Clone)]
pub struct TransactionCoordinator {
    root: PathBuf,
    root_pin: std::sync::Arc<super::cache::directory::Directory>,
    owner: TransactionOwner,
    base_revision: Option<String>,
    allow_session_recovery: bool,
}

impl TransactionCoordinator {
    pub fn new(root: &Path, owner: TransactionOwner) -> Result<Self, String> {
        let root = files::root(root)?;
        let root_pin =
            super::cache::directory::Directory::open(&root, false).map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            root_pin: std::sync::Arc::new(root_pin),
            owner,
            base_revision: None,
            allow_session_recovery: false,
        })
    }

    /// Trusted host input only, obtained from an observed Git revision.
    pub fn with_base_revision(mut self, revision: Option<String>) -> Self {
        self.base_revision = revision;
        self
    }

    /// Capture a bounded proposal. Source files are unchanged; recovery metadata is durable.
    /// Callers authorize reads before invoking this trusted library interface.
    pub fn preview(&self, proposed: Vec<ProposedChange>) -> Result<TransactionSummary, String> {
        self.check_root()?;
        if proposed.is_empty() || proposed.len() > MAX_FILES {
            return Err("transaction requires 1..64 files".into());
        }
        let mut changes = Vec::new();
        let mut paths = std::collections::BTreeSet::new();
        let mut bytes = 0usize;
        for change in proposed {
            let path = files::normalize(&self.root, &change.path)?;
            let key = if cfg!(windows) {
                path.to_ascii_lowercase()
            } else {
                path.clone()
            };
            if !paths.insert(key) {
                return Err("duplicate transaction target".into());
            }
            if change
                .bytes
                .as_ref()
                .is_some_and(|b| b.len() > MAX_FILE_BYTES)
            {
                return Err("transaction file exceeds byte limit".into());
            }
            let (before, before_bytes) = files::capture(&self.root, &path)?;
            if change
                .expected
                .as_ref()
                .is_some_and(|expected| expected != &before)
            {
                return Err(format!(
                    "conflict: {path} changed while preparing the edit; no source files changed"
                ));
            }
            bytes = bytes
                .saturating_add(before.macos_acl.as_ref().map_or(0, Vec::len) * 2)
                .saturating_add(before_bytes.as_ref().map_or(0, Vec::len))
                .saturating_add(before.streams.values().map(Vec::len).sum::<usize>() * 2)
                .saturating_add(
                    before
                        .xattrs
                        .iter()
                        .map(|(name, value)| name.len() + value.len())
                        .sum::<usize>()
                        * 2,
                )
                .saturating_add(change.bytes.as_ref().map_or(0, Vec::len));
            if bytes > MAX_TRANSACTION_BYTES {
                return Err("transaction exceeds byte limit".into());
            }
            let proposed = if change.bytes.is_some() {
                Image {
                    hash: change
                        .bytes
                        .as_ref()
                        .map(|b| super::checkpoints::compute_sha256(b)),
                    identity: None,
                    mode: before.mode.or(Some(0o644)),
                    access: before.access.clone(),
                    unix_owner: before.unix_owner,
                    macos_acl: before.macos_acl.clone(),
                    xattrs: before.xattrs.clone(),
                    streams: before.streams.clone(),
                }
            } else {
                Image::missing()
            };
            changes.push(Change {
                path,
                before,
                before_bytes,
                proposed,
                proposed_bytes: change.bytes,
                staged_name: None,
                restored: None,
                restore_name: None,
            });
        }
        let summary = TransactionSummary {
            id: uuid::Uuid::new_v4().to_string(),
            owner: self.owner.clone(),
            workspace: self.root.to_string_lossy().into_owned(),
            workspace_identity: self.root_pin.identity().map_err(|e| e.to_string())?,
            base_revision: self.base_revision.clone(),
            affected_files: changes.iter().map(|c| c.path.clone()).collect(),
            before_hashes: changes
                .iter()
                .map(|c| (c.path.clone(), c.before.hash.clone()))
                .collect(),
            proposed_hashes: changes
                .iter()
                .map(|c| (c.path.clone(), c.proposed.hash.clone()))
                .collect(),
            applied_hashes: BTreeMap::new(),
            state: TransactionState::Previewed,
            verification_state: "unverified".into(),
            verification: None,
            commit_revision: None,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 1,
            conflict: None,
            warnings: Vec::new(),
        };
        let record = Record {
            schema: 1,
            summary,
            changes,
        };
        self.locked(|store| {
            store.save(&record)?;
            Ok(record.summary.clone())
        })
    }

    pub fn status(&self, id: &str) -> Result<TransactionSummary, String> {
        self.refresh_verification(id)
    }

    /// Host-only discovery for resumed sessions. Records supply candidates, not
    /// authority: begin_verification rechecks ownership and live read policy.
    pub(crate) fn verification_candidates(&self) -> Result<Vec<String>, String> {
        self.check_root()?;
        let directory =
            match super::cache::directory::Directory::open(&self.root.join(STORE_NAME), false) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error.to_string()),
            };
        let store = Store { directory };
        let mut names = store.directory.names().map_err(|e| e.to_string())?;
        names.retain(|name| name.ends_with(".json") && name != "active.json");
        if names.len() > MAX_RECORDS {
            return Err("transaction record capacity exceeded".into());
        }
        names.sort();
        let mut remaining = super::checkpoints::MAX_TASK_BLOBS_BYTES;
        let mut ids = Vec::new();
        for name in names {
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if remaining == 0 {
                break;
            }
            if let Ok(record) = store.load_bounded(
                id,
                &self.root,
                &self.owner,
                self.allow_session_recovery,
                &mut remaining,
            ) {
                if matches!(
                    record.summary.state,
                    TransactionState::Applied | TransactionState::Verified
                ) {
                    ids.push(id.to_owned());
                }
            }
        }
        Ok(ids)
    }

    fn metadata_status(&self, id: &str) -> Result<TransactionSummary, String> {
        self.check_root()?;
        Ok(Store::open(&self.root)?
            .load(id, &self.root, &self.owner, self.allow_session_recovery)?
            .summary)
    }

    /// Trusted library read: the caller must authorize this path first.
    pub fn snapshot(&self, path: &str) -> Result<SourceSnapshot, String> {
        self.check_root()?;
        let path = files::normalize(&self.root, path)?;
        let (image, bytes) = files::capture(&self.root, &path)?;
        Ok(SourceSnapshot { path, image, bytes })
    }

    // Host-only projection from durable, hash-validated records. Never recapture
    // live files after mutation: that could attribute another actor's later edit.
    fn effect_reports(
        &self,
        id: &str,
    ) -> Result<Vec<super::effects::OwnedFileEffectReport>, String> {
        use super::effects::{FileEffectKind, OwnedFileEffect, OwnedFileEffectReport};
        let record = Store::open(&self.root)?.load(
            id,
            &self.root,
            &self.owner,
            self.allow_session_recovery,
        )?;
        Ok(record
            .changes
            .into_iter()
            .map(|change| {
                let kind = if change.before.hash.is_none() {
                    FileEffectKind::Created
                } else if change.proposed.hash.is_none() {
                    FileEffectKind::Deleted
                } else {
                    FileEffectKind::Modified
                };
                let mut effect = OwnedFileEffect::new(
                    id,
                    self.owner.agent_id,
                    record.summary.sequence,
                    change.path,
                    kind,
                    self.owner.task_id.unwrap_or_default(),
                );
                effect.before_blob = change.before.hash;
                effect.after_blob = change.proposed.hash;
                effect.before_mode = change.before.mode;
                effect.after_mode = change.proposed.mode;
                OwnedFileEffectReport {
                    effect,
                    before_bytes: change.before_bytes,
                    after_bytes: change.proposed_bytes,
                }
            })
            .collect())
    }

    fn rollback_effect_reports(
        &self,
        id: &str,
    ) -> Result<Vec<super::effects::OwnedFileEffectReport>, String> {
        use super::effects::FileEffectKind;
        let record = Store::open(&self.root)?.load(
            id,
            &self.root,
            &self.owner,
            self.allow_session_recovery,
        )?;
        let restored: std::collections::BTreeSet<_> = record
            .changes
            .iter()
            .filter(|change| change.restored.is_some())
            .map(|change| change.path.as_str())
            .collect();
        Ok(self
            .effect_reports(id)?
            .into_iter()
            .filter_map(|mut report| {
                if !restored.contains(report.effect.path.as_str()) {
                    return None;
                }
                std::mem::swap(&mut report.before_bytes, &mut report.after_bytes);
                std::mem::swap(
                    &mut report.effect.before_blob,
                    &mut report.effect.after_blob,
                );
                std::mem::swap(
                    &mut report.effect.before_mode,
                    &mut report.effect.after_mode,
                );
                report.effect.operation_id = format!("{id}:rollback");
                report.effect.kind = match report.effect.kind {
                    FileEffectKind::Created => FileEffectKind::Deleted,
                    FileEffectKind::Deleted => FileEffectKind::Created,
                    kind => kind,
                };
                Some(report)
            })
            .collect())
    }

    pub fn apply(
        &self,
        id: &str,
        authority: Authority<'_>,
        abort: Option<&AtomicBool>,
    ) -> Result<TransactionSummary, String> {
        self.locked(|store| {
            self.check_legacy_journals()?;
            if store.active()?.is_some() { return Err("an incomplete transaction requires explicit recovery".into()); }
            let mut record = store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if record.summary.state != TransactionState::Previewed { return Err("transaction is not previewed; application cannot be replayed".into()); }
            for change in &record.changes {
                cancelled(abort)?;
                authority(&self.root.join(&change.path))?;
                if files::capture(&self.root,&change.path)?.0 != change.before {
                    let reason = format!("conflict: {} changed since preview; no source files changed",change.path);
                    return self.conflict(store,&mut record,reason);
                }
            }
            for index in 0..record.changes.len() {
                let change = &mut record.changes[index];
                match files::stage(&self.root,&change.path,change.proposed_bytes.as_deref(),&change.before) {
                    Ok((image,name)) => { change.proposed=image; change.staged_name=name; }
                    Err(error) => { self.cleanup(&record); return Err(error); }
                }
            }
            // Stage identities are persisted before a source entry is replaced. Recovery can
            // identify a completed rename even if the host dies before recording its result.
            if let Err(error) = store.begin(id) { self.cleanup(&record); return Err(error); }
            transition(&mut record,TransactionState::Applying);
            if let Err(error) = store.save(&record) { self.cleanup(&record); return Err(format!("journal persistence failed before mutation: {error}")); }
            for index in 0..record.changes.len() {
                let change = &record.changes[index];
                let mutation = (|| {
                    self.check_root()?;
                    cancelled(abort)?;
                    authority(&self.root.join(&change.path))?;
                    cancelled(abort)?;
                    files::replace(&self.root,&change.path,&change.before,change.staged_name.as_deref(),&change.proposed)
                })();
                if let Err(error) = mutation {
                    return match self.rollback_locked(store,&mut record,authority,None) {
                        Ok(_) => Err(format!("transaction mutation failed and owned changes were rolled back: {error}")),
                        Err(recovery) => Err(format!("transaction mutation failed: {error}; recovery retained: {recovery}")),
                    };
                }
                record.summary.applied_hashes.insert(change.path.clone(),change.proposed.hash.clone());
            }
            for change in &record.changes {
                if files::capture(&self.root,&change.path)?.0 != change.proposed {
                    let reason = format!("conflict: {} changed during application; journal retained",change.path);
                    return self.conflict(store,&mut record,reason);
                }
            }
            transition(&mut record,TransactionState::Applied);
            store.save(&record)?;
            store.finish(id)?;
            self.cleanup(&record);
            Ok(record.summary)
        })
    }

    /// Also recovers an interrupted Applying/RollingBack record. Current permission and
    /// complete preflight are required; no Git reset/checkout or unconditional writes.
    pub fn rollback(
        &self,
        id: &str,
        authority: Authority<'_>,
        abort: Option<&AtomicBool>,
    ) -> Result<TransactionSummary, String> {
        self.locked(|store| {
            self.check_legacy_journals()?;
            if store.active()?.is_some_and(|active| active != id) {
                return Err("another incomplete transaction requires recovery".into());
            }
            let mut record =
                store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            self.rollback_locked(store, &mut record, authority, abort)
        })
    }

    fn rollback_locked(
        &self,
        store: &Store,
        record: &mut Record,
        authority: Authority<'_>,
        abort: Option<&AtomicBool>,
    ) -> Result<TransactionSummary, String> {
        if matches!(
            record.summary.state,
            TransactionState::Committed | TransactionState::RolledBack | TransactionState::Draft
        ) {
            return Err("transaction state does not allow rollback".into());
        }
        let mut restore = Vec::new();
        for change in &record.changes {
            cancelled(abort)?;
            authority(&self.root.join(&change.path))?;
            let current = files::capture(&self.root, &change.path)?.0;
            if current == change.before || change.restored.as_ref() == Some(&current) {
                continue;
            }
            if current != change.proposed
                || change.proposed.identity.is_none() && change.proposed.hash.is_some()
            {
                let reason = format!("conflict: {} no longer has transaction-owned bytes and identity; no rollback writes",change.path);
                return self.conflict(store, record, reason);
            }
            restore.push((change.path.clone(), current));
        }
        for (path, _) in &restore {
            let change = record
                .changes
                .iter_mut()
                .find(|c| &c.path == path)
                .expect("validated path");
            files::cleanup(&self.root, path, change.restore_name.as_deref());
            let (image, name) = match files::stage(
                &self.root,
                path,
                change.before_bytes.as_deref(),
                &change.before,
            ) {
                Ok(staged) => staged,
                Err(error) => {
                    self.cleanup(record);
                    return Err(error);
                }
            };
            change.restored = Some(image);
            change.restore_name = name;
        }
        let prepare = store.active().and_then(|active| {
            if active.is_none() {
                store.begin(&record.summary.id)
            } else {
                Ok(())
            }
        });
        if let Err(error) = prepare {
            self.cleanup(record);
            return Err(error);
        }
        transition(record, TransactionState::RollingBack);
        if let Err(error) = store.save(record) {
            self.cleanup(record);
            return Err(error);
        }
        for (path, current) in restore.into_iter().rev() {
            self.check_root()?;
            let change = record
                .changes
                .iter()
                .find(|c| c.path == path)
                .expect("validated path");
            let result = cancelled(abort)
                .and_then(|_| authority(&self.root.join(&path)))
                .and_then(|_| cancelled(abort))
                .and_then(|_| {
                    files::replace(
                        &self.root,
                        &path,
                        &current,
                        change.restore_name.as_deref(),
                        change.restored.as_ref().expect("staged restore"),
                    )
                });
            if let Err(error) = result {
                return self.conflict(
                    store,
                    record,
                    format!("rollback incomplete; journal retained: {error}"),
                );
            }
        }
        for change in &record.changes {
            let current = files::capture(&self.root, &change.path)?.0;
            if current != change.before && change.restored.as_ref() != Some(&current) {
                let reason = format!(
                    "conflict: {} changed during rollback; journal retained",
                    change.path
                );
                return self.conflict(store, record, reason);
            }
        }
        transition(record, TransactionState::RolledBack);
        record.summary.verification_state = "invalidated by rollback".into();
        store.save(record)?;
        store.finish(&record.summary.id)?;
        self.cleanup(record);
        Ok(record.summary.clone())
    }

    fn locked<T>(&self, operation: impl FnOnce(&Store) -> Result<T, String>) -> Result<T, String> {
        self.check_root()?;
        crate::file_mutation_queue::with_file_mutation_queue(&self.root, || {
            let store = Store::open(&self.root)?;
            let _lease = store
                .directory
                .lease()
                .map_err(|e| format!("workspace mutation lane is busy: {e}"))?;
            store.directory.check_current().map_err(|e| e.to_string())?;
            operation(&store)
        })
    }
    fn check_root(&self) -> Result<(), String> {
        self.root_pin
            .check_current()
            .map_err(|e| format!("transaction workspace changed: {e}"))
    }
    fn conflict<T>(&self, store: &Store, record: &mut Record, reason: String) -> Result<T, String> {
        transition(record, TransactionState::Conflicted);
        record.summary.conflict = Some(reason.clone());
        record.summary.verification_state = "conflicted".into();
        store.save(record)?;
        Err(reason)
    }
    fn cleanup(&self, record: &Record) {
        for change in &record.changes {
            files::cleanup(&self.root, &change.path, change.staged_name.as_deref());
            files::cleanup(&self.root, &change.path, change.restore_name.as_deref());
        }
    }
    fn check_legacy_journals(&self) -> Result<(), String> {
        for name in [
            crate::apply_patch::JOURNAL_FILE_NAME,
            crate::apply_patch::LEGACY_JOURNAL_FILE_NAME,
            crate::apply_patch::REWIND_JOURNAL_FILE_NAME,
            crate::apply_patch::LEGACY_REWIND_JOURNAL_FILE_NAME,
        ] {
            match std::fs::symlink_metadata(self.root.join(name)) {
                Ok(_) => {
                    return Err(
                        "an existing patch or rewind journal requires explicit authorized recovery"
                            .into(),
                    )
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
}

fn transition(record: &mut Record, state: TransactionState) {
    record.summary.state = state;
    record.summary.sequence = record.summary.sequence.saturating_add(1);
}
fn cancelled(abort: Option<&AtomicBool>) -> Result<(), String> {
    if abort.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err("transaction cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn verification_discovery_preserves_owner_boundaries_and_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let owner = TransactionOwner {
            session_id: Some("discovery-session".into()),
            ..TransactionOwner::default()
        };
        let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
        assert!(manager.verification_candidates().unwrap().is_empty());
        assert!(!root.path().join(STORE_NAME).exists());
        let preview = manager
            .preview(vec![ProposedChange::write("source", b"after".to_vec())])
            .unwrap();
        assert!(manager.verification_candidates().unwrap().is_empty());
        manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
        assert_eq!(
            manager.verification_candidates().unwrap(),
            [preview.id.clone()]
        );
        for boundary in ["same-session", "foreign-session", "task", "parent", "graph"] {
            let mut resumed = owner.clone();
            resumed.agent_id = super::super::AgentId::new();
            match boundary {
                "foreign-session" => resumed.session_id = Some("foreign".into()),
                "task" => resumed.task_id = Some(super::super::TaskId::new()),
                "parent" => resumed.parent_agent_id = Some(owner.agent_id),
                "graph" => resumed.graph_node = Some("node".into()),
                _ => (),
            }
            let mut reader = TransactionCoordinator::new(root.path(), resumed).unwrap();
            assert!(reader.verification_candidates().unwrap().is_empty());
            reader.allow_session_recovery = true;
            assert_eq!(
                reader.verification_candidates().unwrap().len(),
                usize::from(boundary == "same-session"),
                "{boundary}"
            );
        }
        manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
        assert!(manager.verification_candidates().unwrap().is_empty());
    }

    #[test]
    fn verification_discovery_reads_obey_shared_byte_budget() {
        let root = tempfile::tempdir().unwrap();
        let owner = TransactionOwner::default();
        let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
        let preview = manager
            .preview(vec![ProposedChange::write("source", b"after".to_vec())])
            .unwrap();
        let store = Store::open(root.path()).unwrap();
        let size = store
            .directory
            .file(&format!("{}.json", preview.id), false)
            .unwrap()
            .metadata()
            .unwrap()
            .len();
        let mut remaining = size;
        store
            .load_bounded(&preview.id, &manager.root, &owner, false, &mut remaining)
            .unwrap();
        assert_eq!(remaining, 0);
        assert!(store
            .load_bounded(&preview.id, &manager.root, &owner, false, &mut remaining)
            .is_err());
    }
}
