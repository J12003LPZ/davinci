use super::{files, model::*};
use crate::runtime::cache::directory::Directory;
use crate::runtime::checkpoints::compute_sha256;
use std::io::{Read, Write};
use std::path::Path;

pub(super) struct Store {
    pub directory: Directory,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: Directory::open(&root.join(STORE_NAME), true)
                .map_err(|e| format!("transaction store: {e}"))?,
        })
    }
    pub fn load(
        &self,
        id: &str,
        root: &Path,
        owner: &TransactionOwner,
        allow_session_recovery: bool,
    ) -> Result<Record, String> {
        let mut remaining = MAX_RECORD_BYTES;
        self.load_bounded(id, root, owner, allow_session_recovery, &mut remaining)
    }
    pub fn load_bounded(
        &self,
        id: &str,
        root: &Path,
        owner: &TransactionOwner,
        allow_session_recovery: bool,
        remaining: &mut u64,
    ) -> Result<Record, String> {
        let id = valid_id(id)?;
        let mut file = self
            .directory
            .file(&format!("{id}.json"), false)
            .map_err(|e| format!("transaction record: {e}"))?;
        let limit = MAX_RECORD_BYTES.min(*remaining);
        if file.metadata().map_err(|e| e.to_string())?.len() > limit {
            return Err("transaction record exceeds limit".into());
        }
        let mut bytes = Vec::new();
        let read = (&mut file).take(limit + 1).read_to_end(&mut bytes);
        *remaining = remaining.saturating_sub(bytes.len() as u64);
        read.map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("transaction record exceeds limit".into());
        }
        let record: Record = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid transaction record: {e}"))?;
        let original = &record.summary.owner;
        let resumed_owner = allow_session_recovery
            && owner.session_id.as_ref().is_some_and(|id| !id.is_empty())
            && original.session_id == owner.session_id
            && original.task_id == owner.task_id
            && original.parent_agent_id.is_none()
            && owner.parent_agent_id.is_none()
            && original.graph_node.is_none()
            && owner.graph_node.is_none();
        if record.schema != 1
            || record.summary.id != id
            || (original != owner && !resumed_owner)
            || Path::new(&record.summary.workspace) != root
        {
            return Err("transaction workspace or owner mismatch".into());
        }
        if Directory::open(root, false)
            .and_then(|d| d.identity())
            .map_err(|e| e.to_string())?
            != record.summary.workspace_identity
        {
            return Err("transaction workspace identity changed".into());
        }
        validate(&record, root)?;
        Ok(record)
    }
    pub fn save(&self, record: &Record) -> Result<(), String> {
        let id = valid_id(&record.summary.id)?;
        let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err("transaction record exceeds limit".into());
        }
        let name = format!("{id}.json");
        let names = self.directory.names().map_err(|e| e.to_string())?;
        let records: Vec<_> = names
            .iter()
            .filter(|s| s.ends_with(".json") && s.as_str() != "active.json")
            .collect();
        if records.len() >= MAX_RECORDS && !records.contains(&&name) {
            return Err("transaction record capacity reached; preserve or archive completed records before continuing".into());
        }
        let mut total = bytes.len() as u64;
        for other in records {
            if other == &name {
                continue;
            }
            total = total.saturating_add(
                self.directory
                    .file(other, false)
                    .and_then(|f| f.metadata())
                    .map_err(|e| e.to_string())?
                    .len(),
            );
        }
        if total > super::super::checkpoints::MAX_TASK_BLOBS_BYTES {
            return Err("durable transaction storage capacity reached".into());
        }
        self.atomic(&name, &bytes)
    }
    fn atomic(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.atomic_with(name, bytes, false, |file, bytes| file.write_all(bytes))
    }
    fn atomic_with(
        &self,
        name: &str,
        bytes: &[u8],
        create_only: bool,
        write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    ) -> Result<(), String> {
        let temp = format!("{}.tmp", uuid::Uuid::new_v4());
        let result = (|| {
            let mut file = self
                .directory
                .file(&temp, true)
                .map_err(|e| e.to_string())?;
            #[cfg(test)]
            failure_tests::inject_write_failure(&mut file, bytes).map_err(|e| e.to_string())?;
            write(&mut file, bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            let existing = if create_only {
                false
            } else {
                match self.directory.file(name, false) {
                    Ok(_) => true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(e) => return Err(e.to_string()),
                }
            };
            self.directory
                .replace_source(&temp, name, existing)
                .map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = self.directory.remove(&temp);
        }
        result
    }
    pub fn active(&self) -> Result<Option<String>, String> {
        let file = match self.directory.file("active.json", false) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut id = String::new();
        file.take(64)
            .read_to_string(&mut id)
            .map_err(|e| e.to_string())?;
        valid_id(&id)?;
        Ok(Some(id))
    }
    pub fn begin(&self, id: &str) -> Result<(), String> {
        valid_id(id)?;
        self.atomic_with("active.json", id.as_bytes(), true, |file, bytes| {
            file.write_all(bytes)
        })
        .map_err(|e| format!("active transaction marker could not be published: {e}"))
    }
    pub fn finish(&self, id: &str) -> Result<(), String> {
        if self.active()?.as_deref() == Some(id) {
            self.directory
                .remove_source("active.json")
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> Result<String, String> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "invalid transaction id")?;
    if parsed.to_string() != id {
        return Err("invalid transaction id".into());
    }
    Ok(id.into())
}

fn validate(record: &Record, root: &Path) -> Result<(), String> {
    super::verification::validate(record)?;
    super::commit::validate(record)?;
    if record.changes.is_empty() || record.changes.len() > MAX_FILES {
        return Err("invalid transaction file count".into());
    }
    if record.summary.sequence == 0
        || record
            .summary
            .owner
            .session_id
            .as_ref()
            .is_some_and(|s| s.len() > 1024)
        || record
            .summary
            .owner
            .graph_node
            .as_ref()
            .is_some_and(|s| s.len() > 1024)
        || record
            .summary
            .base_revision
            .as_ref()
            .is_some_and(|s| s.len() > 128)
        || record.summary.verification_state.len() > 1024
        || record
            .summary
            .conflict
            .as_ref()
            .is_some_and(|s| s.len() > 8192)
        || record.summary.warnings.len() > 8
        || record.summary.warnings.iter().any(|s| s.len() > 8192)
    {
        return Err("invalid transaction provenance".into());
    }
    let mut paths = std::collections::BTreeSet::new();
    let mut bytes = 0usize;
    for change in &record.changes {
        let path = files::normalize(root, &change.path)?;
        let key = if cfg!(windows) {
            path.to_ascii_lowercase()
        } else {
            path.clone()
        };
        if path != change.path || !paths.insert(key) {
            return Err("invalid duplicate transaction path".into());
        }
        for (image, content) in [
            (&change.before, &change.before_bytes),
            (&change.proposed, &change.proposed_bytes),
        ] {
            validate_image(image)?;
            bytes = bytes.saturating_add(image.macos_acl.as_ref().map_or(0, Vec::len));
            bytes = bytes.saturating_add(image.streams.values().map(Vec::len).sum::<usize>());
            bytes = bytes.saturating_add(
                image
                    .xattrs
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>(),
            );
            if content.as_ref().map(|v| compute_sha256(v)) != image.hash {
                return Err("transaction preimage/proposal hash mismatch".into());
            }
            if let Some(content) = content {
                if content.len() > MAX_FILE_BYTES {
                    return Err("transaction blob exceeds limit".into());
                }
                bytes = bytes.saturating_add(content.len());
            }
        }
        if let Some(restored) = &change.restored {
            validate_image(restored)?;
            for (field, matches) in [
                ("content", restored.hash == change.before.hash),
                ("mode", restored.mode == change.before.mode),
                ("access", restored.access == change.before.access),
                ("owner", restored.unix_owner == change.before.unix_owner),
                ("macos_acl", restored.macos_acl == change.before.macos_acl),
                ("xattrs", restored.xattrs == change.before.xattrs),
                ("streams", restored.streams == change.before.streams),
            ] {
                if !matches {
                    return Err(format!("transaction restore image mismatch: {field}"));
                }
            }
        }
        if change.before.hash.is_some() && change.before.identity.is_none() {
            return Err("transaction preimage identity is missing".into());
        }
        for temp in [
            change.staged_name.as_deref(),
            change.restore_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let id = temp
                .strip_prefix(".davinci-txn-")
                .and_then(|s| s.strip_suffix(".tmp"))
                .ok_or("invalid transaction staging name")?;
            valid_id(id)?;
        }
    }
    if bytes > MAX_TRANSACTION_BYTES {
        return Err("transaction exceeds byte limit".into());
    }
    if record.summary.affected_files
        != record
            .changes
            .iter()
            .map(|c| c.path.clone())
            .collect::<Vec<_>>()
    {
        return Err("transaction file summary mismatch".into());
    }
    if record.summary.before_hashes.len() != record.changes.len()
        || record.summary.proposed_hashes.len() != record.changes.len()
        || record
            .summary
            .applied_hashes
            .iter()
            .any(|(path, hash)| record.summary.proposed_hashes.get(path) != Some(hash))
    {
        return Err("transaction hash summary mismatch".into());
    }
    for change in &record.changes {
        if record.summary.before_hashes.get(&change.path) != Some(&change.before.hash)
            || record.summary.proposed_hashes.get(&change.path) != Some(&change.proposed.hash)
        {
            return Err("transaction hash summary mismatch".into());
        }
    }
    Ok(())
}

fn validate_image(image: &Image) -> Result<(), String> {
    if let Some(acl) = &image.macos_acl {
        super::macos_acl::validate(acl)?;
        if !cfg!(target_os = "macos") {
            return Err("macOS transaction ACL on unsupported platform".into());
        }
    }
    super::unix_xattrs::validate(&image.xattrs)?;
    #[cfg(not(unix))]
    if !image.xattrs.is_empty() {
        return Err("Unix transaction attributes on unsupported platform".into());
    }
    if image.unix_owner.is_some_and(|ids| ids.contains(&u32::MAX)) {
        return Err("invalid transaction Unix ownership".into());
    }
    #[cfg(not(unix))]
    if image.unix_owner.is_some() {
        return Err("Unix transaction ownership on unsupported platform".into());
    }
    #[cfg(windows)]
    super::windows_streams::validate(&image.streams)?;
    #[cfg(not(windows))]
    if !image.streams.is_empty() {
        return Err("Windows transaction streams on unsupported platform".into());
    }
    if image.hash.is_none()
        && (image.identity.is_some()
            || image.mode.is_some()
            || image.access.is_some()
            || image.unix_owner.is_some()
            || image.macos_acl.is_some()
            || !image.xattrs.is_empty()
            || !image.streams.is_empty())
    {
        return Err("invalid missing transaction image".into());
    }
    if image.access.as_ref().is_some_and(|s| {
        s.len() > 64 * 1024 || !(s.starts_with("D:") || s.starts_with("O:")) || s.contains('\0')
    }) {
        return Err("invalid transaction access permissions".into());
    }
    let max_mode = if cfg!(unix) { 0o7777 } else { 0o777 };
    if image.hash.is_some() && image.mode.is_none_or(|mode| mode > max_mode) {
        return Err("invalid transaction file mode".into());
    }
    if image.identity.as_ref().is_some_and(|id| {
        id.len() > 128 || id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit() || b == b':')
    }) {
        return Err("invalid transaction file identity".into());
    }
    Ok(())
}

#[cfg(test)]
mod failure_tests {
    use super::super::{ProposedChange, TransactionCoordinator, TransactionState};
    use super::*;

    #[test]
    fn transaction_restore_mismatch_identifies_field_without_metadata_values() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        std::fs::write(root.join("file.txt"), b"private contents").unwrap();
        let owner = TransactionOwner::default();
        let manager = TransactionCoordinator::new(&root, owner.clone()).unwrap();
        let preview = manager
            .preview(vec![ProposedChange::write("file.txt", b"after".to_vec())])
            .unwrap();
        let store = Store::open(&root).unwrap();
        let mut record = store.load(&preview.id, &root, &owner, false).unwrap();
        let mut restored = record.changes[0].before.clone();
        restored.mode = Some(restored.mode.unwrap() ^ 0o200);
        record.changes[0].restored = Some(restored);
        assert_eq!(
            validate(&record, &root).unwrap_err(),
            "transaction restore image mismatch: mode"
        );
    }

    #[test]
    fn transaction_unix_ownership_validation_rejects_invalid_images() {
        let mut image = Image::missing();
        image.unix_owner = Some([1000, 100]);
        assert!(validate_image(&image).is_err());
        image.hash = Some("hash".into());
        image.identity = Some("1:2".into());
        image.mode = Some(0o644);
        image.unix_owner = Some([u32::MAX, 100]);
        assert!(validate_image(&image).is_err());
        image.unix_owner = Some([1000, u32::MAX]);
        assert!(validate_image(&image).is_err());
        image.unix_owner = Some([1000, 100]);
        assert_eq!(validate_image(&image).is_ok(), cfg!(unix));
    }

    thread_local! {
        static FAIL_WRITE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    pub(super) fn inject_write_failure(
        file: &mut std::fs::File,
        bytes: &[u8],
    ) -> std::io::Result<()> {
        let fail = FAIL_WRITE.with(|remaining| {
            let value = remaining.get();
            remaining.set(value.saturating_sub(1));
            value == 1
        });
        if fail {
            file.write_all(&bytes[..bytes.len().min(3)])?;
            return Err(std::io::Error::other("injected partial journal write"));
        }
        Ok(())
    }

    fn fail_write<T>(index: usize, operation: impl FnOnce() -> T) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FAIL_WRITE.with(|remaining| remaining.set(0));
            }
        }
        FAIL_WRITE.with(|remaining| remaining.set(index));
        let _reset = Reset;
        operation()
    }

    #[test]
    fn transaction_journal_failure_windows_remain_recoverable() {
        for failed_write in 1..=3 {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("a.txt"), b"before").unwrap();
            let owner = TransactionOwner::default();
            let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
                .unwrap();
            let result = fail_write(failed_write, || {
                manager.apply(&preview.id, &|_| Ok(()), None)
            });
            assert!(result.is_err(), "failure point {failed_write}");
            assert_eq!(
                std::fs::read(root.path().join("a.txt")).unwrap(),
                if failed_write == 3 {
                    b"after".as_slice()
                } else {
                    b"before".as_slice()
                }
            );
            let store = Store::open(&root.path().canonicalize().unwrap()).unwrap();
            assert_eq!(store.active().unwrap().is_some(), failed_write != 1);
            assert_eq!(
                manager.status(&preview.id).unwrap().state,
                if failed_write == 3 {
                    TransactionState::Applying
                } else {
                    TransactionState::Previewed
                }
            );
            let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
            recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(std::fs::read(root.path().join("a.txt")).unwrap(), b"before");
            assert_eq!(
                recovered.status(&preview.id).unwrap().state,
                TransactionState::RolledBack
            );
            assert!(store.active().unwrap().is_none());
        }
    }

    #[test]
    fn transaction_marker_partial_write_is_not_published() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().canonicalize().unwrap()).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let result = store.atomic_with("active.json", id.as_bytes(), true, |file, bytes| {
            file.write_all(&bytes[..5])?;
            Err(std::io::Error::other("injected storage write failure"))
        });
        assert!(result.is_err());
        assert!(store.active().unwrap().is_none());
        assert!(store
            .directory
            .names()
            .unwrap()
            .iter()
            .all(|name| !name.ends_with(".tmp")));
        store.begin(&id).unwrap();
        assert_eq!(store.active().unwrap().as_deref(), Some(id.as_str()));
        let other = uuid::Uuid::new_v4().to_string();
        assert!(store.begin(&other).is_err());
        assert_eq!(store.active().unwrap().as_deref(), Some(id.as_str()));
        store.finish(&id).unwrap();
        assert!(store.active().unwrap().is_none());
    }

    #[test]
    fn transaction_rollback_journal_failures_recover_without_orphan_stages() {
        for failed_write in 1..=3 {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("a.txt"), b"before").unwrap();
            let owner = TransactionOwner::default();
            let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
                .unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            let error = fail_write(failed_write, || {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            })
            .unwrap_err();
            assert!(error.contains("injected"), "{error}");
            assert_eq!(
                std::fs::read(root.path().join("a.txt")).unwrap(),
                if failed_write == 3 {
                    b"before".as_slice()
                } else {
                    b"after".as_slice()
                }
            );
            let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
            recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(std::fs::read(root.path().join("a.txt")).unwrap(), b"before");
            assert_eq!(
                recovered.status(&preview.id).unwrap().state,
                TransactionState::RolledBack
            );
            assert!(Store::open(&root.path().canonicalize().unwrap())
                .unwrap()
                .active()
                .unwrap()
                .is_none());
            assert!(
                std::fs::read_dir(root.path())
                    .unwrap()
                    .map(Result::unwrap)
                    .all(|entry| !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".davinci-txn-")),
                "orphan source stage after failure point {failed_write}"
            );
        }
    }

    #[test]
    fn transaction_record_partial_write_preserves_previous_journal() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().canonicalize().unwrap()).unwrap();
        let name = "fixture.json";
        store.atomic(name, b"previous durable journal").unwrap();
        assert!(store
            .atomic_with(name, b"replacement journal", false, |file, bytes| {
                file.write_all(&bytes[..3])?;
                Err(std::io::Error::other("injected storage write failure"))
            })
            .is_err());
        let mut bytes = Vec::new();
        store
            .directory
            .file(name, false)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"previous durable journal");
        assert!(store
            .directory
            .names()
            .unwrap()
            .iter()
            .all(|name| !name.ends_with(".tmp")));
    }
}
