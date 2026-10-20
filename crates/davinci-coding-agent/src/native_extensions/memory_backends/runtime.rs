//! The live state of the remote projection for one [`VectorMemory`]: the
//! client, whether Qdrant is currently considered down, and why.
//!
//! After a failure the projection is skipped for the configured backoff so a
//! stopped Qdrant costs one timeout, not one per prompt (the same shape as the
//! dense-embedding backoff). Callers fall back to local search and report the
//! state through [`ProjectionRuntime::health`].
//!
//! [`VectorMemory`]: crate::native_extensions::vector_memory::VectorMemory

use super::pending_seal;
use super::qdrant::QdrantProjection;
use super::{MemoryProjectionBackend, ProjectionError, VectorProjection};
use crate::native_extensions::vector_memory::{sha256_hex, VectorMemoryConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Stable code shown when the remote could not be used and local search ran.
pub const QDRANT_OFFLINE: &str = "QDRANT_OFFLINE";
/// Stable code for a remote that answered but must not be used (wrong
/// collection shape, bad key, remote URL without consent).
pub const QDRANT_REFUSED: &str = "QDRANT_REFUSED";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionState {
    /// Local backend: the remote is not in use.
    Off,
    /// Chosen, not contacted yet.
    Unchecked,
    Ready,
    Offline,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionHealth {
    pub state: ProjectionState,
    pub code: Option<&'static str>,
    pub detail: String,
}

/// Which record ids the remote collection is known to hold. A missing,
/// unreadable or foreign-collection file reads as empty: the next reconcile
/// re-sends everything, which is idempotent.
#[derive(Default, Serialize, Deserialize)]
struct SyncFile {
    #[serde(default)]
    collection: String,
    #[serde(default)]
    synced: BTreeSet<String>,
    /// Clears confirmed locally that have not reached the remote. Each names
    /// the collection it applies to, so a change of embedding model or repo id
    /// (which renames the collection) does not forget it. They survive a
    /// restart and are dropped only after the remote delete worked.
    #[serde(default)]
    pending_clears: Vec<PendingClear>,
    /// Bumped by every clear. A catch-up that works on a snapshot compares it
    /// before and after uploading and backs off if a clear happened meanwhile.
    #[serde(default)]
    clear_epoch: u64,
}

/// A remote delete the user already confirmed locally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingClear {
    /// The collection the clear applies to.
    pub collection: String,
    /// The repo id the ids were written under. It follows the checked-out
    /// branch and git config, so it can differ by the time the clear is
    /// retried; the delete filter must use this one. Empty in a file written
    /// before this field existed: the current repo id is used then.
    #[serde(default)]
    pub repo_id: String,
    /// Exactly the records the clear removed, taken when it was confirmed:
    /// the ledger plus the local records. Later uploads from this or another
    /// clone of the same repository are not touched when it runs.
    pub ids: Vec<String>,
    /// HMAC-SHA256 over the three fields above and the path of this file, under
    /// a per-user key kept outside every repository. The file sits in the
    /// working tree, so anyone who controls a checkout controls its contents;
    /// an entry without a valid MAC is dropped, not run.
    #[serde(default)]
    pub mac: String,
}

#[derive(Default)]
struct SyncState {
    loaded_from: Option<PathBuf>,
    file: SyncFile,
}

struct Inner {
    sync: SyncState,
    injected: Option<Arc<dyn VectorProjection>>,
    cached: Option<(String, Arc<dyn VectorProjection>)>,
    backoff_until: Option<Instant>,
    health: ProjectionHealth,
}

pub struct ProjectionRuntime {
    inner: Mutex<Inner>,
}

impl Default for ProjectionRuntime {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                sync: SyncState::default(),
                injected: None,
                cached: None,
                backoff_until: None,
                health: ProjectionHealth {
                    state: ProjectionState::Off,
                    code: None,
                    detail: String::new(),
                },
            }),
        }
    }
}

impl fmt::Debug for ProjectionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectionRuntime")
            .field("health", &self.health_unchecked())
            .finish()
    }
}

impl ProjectionRuntime {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn health_unchecked(&self) -> ProjectionHealth {
        self.lock().health.clone()
    }

    /// Use this projection instead of building a Qdrant client. For tests.
    pub fn inject(&self, projection: Arc<dyn VectorProjection>) {
        self.lock().injected = Some(projection);
    }

    /// The health to show for this config: `Off` whenever the backend is local,
    /// whatever happened earlier.
    pub fn health(&self, config: &VectorMemoryConfig) -> ProjectionHealth {
        if config.projection.backend != MemoryProjectionBackend::Qdrant {
            return ProjectionHealth {
                state: ProjectionState::Off,
                code: None,
                detail: String::new(),
            };
        }
        let mut health = self.health_unchecked();
        if health.state == ProjectionState::Off {
            health.state = ProjectionState::Unchecked;
        }
        health
    }

    /// Ids the remote holds for `collection`, read from `path` the first time.
    pub fn synced_ids(&self, path: &Path, collection: &str) -> BTreeSet<String> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        inner.sync.file.synced.clone()
    }

    /// Record that these ids reached the remote and these left it.
    pub fn update_synced(
        &self,
        path: &Path,
        collection: &str,
        added: impl IntoIterator<Item = String>,
        removed: impl IntoIterator<Item = String>,
    ) {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        inner.sync.file.synced.extend(added);
        for id in removed {
            inner.sync.file.synced.remove(&id);
        }
        Self::save_sync(&inner.sync, path);
    }

    /// The remote lost this namespace: forget what it held so the next
    /// reconcile sends everything again. A pending clear is kept.
    pub fn forget_synced(&self, path: &Path, collection: &str) {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        inner.sync.file.synced.clear();
        Self::save_sync(&inner.sync, path);
    }

    /// Record, durably, that these ids must leave the remote collection, and
    /// forget the ledger. Errors when the marker cannot be written: the caller
    /// must not confirm the clear then, because nothing would remember to
    /// finish it. With no ids there is nothing to owe, so no marker is kept.
    pub fn begin_clear(
        &self,
        path: &Path,
        collection: &str,
        repo_id: &str,
        ids: Vec<String>,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        let key = Self::signing_key(&ids)?;
        let before = (
            inner.sync.file.synced.clone(),
            inner.sync.file.pending_clears.clone(),
            inner.sync.file.clear_epoch,
        );
        inner.sync.file.synced.clear();
        inner.sync.file.clear_epoch += 1;
        Self::merge_owed(&mut inner.sync.file, collection, repo_id, ids, key, path);
        let saved = Self::save_sync_checked(&inner.sync, path);
        if saved.is_err() {
            inner.sync.file.synced = before.0;
            inner.sync.file.pending_clears = before.1;
            inner.sync.file.clear_epoch = before.2;
        }
        saved
    }

    /// Owe the remote a delete of these ids without touching the ledger or the
    /// epoch: points that were uploaded and could not be taken back out.
    pub fn owe_clear(
        &self,
        path: &Path,
        collection: &str,
        repo_id: &str,
        ids: Vec<String>,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        let key = Self::signing_key(&ids)?;
        let before = inner.sync.file.pending_clears.clone();
        Self::merge_owed(&mut inner.sync.file, collection, repo_id, ids, key, path);
        let saved = Self::save_sync_checked(&inner.sync, path);
        if saved.is_err() {
            inner.sync.file.pending_clears = before;
        }
        saved
    }

    /// The per-user key, made on first use, when there is something to sign.
    /// No key means no clear can be recorded, so the caller refuses the clear.
    fn signing_key(ids: &[String]) -> Result<Option<pending_seal::Key>, String> {
        if ids.is_empty() {
            return Ok(None);
        }
        pending_seal::key_or_create()
            .map(Some)
            .map_err(|error| format!("no per-user key to sign the pending clear with: {error}"))
    }

    fn merge_owed(
        file: &mut SyncFile,
        collection: &str,
        repo_id: &str,
        ids: Vec<String>,
        key: Option<pending_seal::Key>,
        store: &Path,
    ) {
        let Some(key) = key else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        match file
            .pending_clears
            .iter_mut()
            .find(|pending| pending.collection == collection && pending.repo_id == repo_id)
        {
            Some(pending) => {
                let merged: BTreeSet<String> = pending.ids.iter().cloned().chain(ids).collect();
                pending.ids = merged.into_iter().collect();
                pending.mac = pending_seal::sign(&key, pending, store);
            }
            None => {
                let unique: BTreeSet<String> = ids.into_iter().collect();
                let mut entry = PendingClear {
                    collection: collection.to_string(),
                    repo_id: repo_id.to_string(),
                    ids: unique.into_iter().collect(),
                    mac: String::new(),
                };
                entry.mac = pending_seal::sign(&key, &entry, store);
                file.pending_clears.push(entry);
            }
        }
    }

    /// Every clear still owed to the remote, for any collection.
    pub fn pending_clears(&self, path: &Path, collection: &str) -> Vec<PendingClear> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        inner.sync.file.pending_clears.clone()
    }

    /// Whether a confirmed clear still has to reach the remote.
    pub fn pending_clear(&self, path: &Path, collection: &str) -> bool {
        !self.pending_clears(path, collection).is_empty()
    }

    /// How many clears this state has seen.
    pub fn clear_epoch(&self, path: &Path, collection: &str) -> u64 {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        inner.sync.file.clear_epoch
    }

    /// The whole namespace of `collection` was deleted on the remote: drop
    /// every owed clear for it and the ledger. If the file cannot be written
    /// the in-memory state is left as it was, so it never claims more than the
    /// disk does.
    pub fn finish_clear(&self, path: &Path, collection: &str) -> Result<(), String> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        let before = (
            inner.sync.file.synced.clone(),
            inner.sync.file.pending_clears.clone(),
        );
        inner.sync.file.synced.clear();
        inner
            .sync
            .file
            .pending_clears
            .retain(|pending| pending.collection != collection);
        let saved = Self::save_sync_checked(&inner.sync, path);
        if saved.is_err() {
            inner.sync.file.synced = before.0;
            inner.sync.file.pending_clears = before.1;
        }
        saved
    }

    /// One owed clear reached the remote: drop it, and only it. The ledger
    /// belongs to the current collection and is not touched. Same rule on a
    /// failed write as [`Self::finish_clear`].
    pub fn finish_pending(
        &self,
        path: &Path,
        collection: &str,
        done: &PendingClear,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        Self::load_sync(&mut inner.sync, path, collection);
        let before = inner.sync.file.pending_clears.clone();
        inner
            .sync
            .file
            .pending_clears
            .retain(|pending| pending != done);
        let saved = Self::save_sync_checked(&inner.sync, path);
        if saved.is_err() {
            inner.sync.file.pending_clears = before;
        }
        saved
    }

    fn load_sync(sync: &mut SyncState, path: &Path, collection: &str) {
        if sync.loaded_from.as_deref() != Some(path) {
            sync.file = std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            sync.loaded_from = Some(path.to_path_buf());
            if Self::drop_untrusted(&mut sync.file, path, collection) {
                Self::save_sync(sync, path);
            }
        }
        // Another model or repo means another collection: nothing is synced.
        // A clear still owed to the old collection is kept, with the name it
        // applies to.
        if sync.file.collection != collection {
            sync.file = SyncFile {
                collection: collection.to_string(),
                synced: BTreeSet::new(),
                pending_clears: std::mem::take(&mut sync.file.pending_clears),
                clear_epoch: sync.file.clear_epoch,
            };
        }
    }

    /// Keep only the owed clears this user recorded. The file is read from the
    /// working tree, where a repository can ship anything: an entry whose MAC
    /// is missing or wrong is dropped with one warning and never executed.
    /// One exception keeps files from before signing working: an entry with no
    /// MAC and no repo id, for the current collection, which can only act on
    /// the current repo id in the current collection, i.e. this checkout.
    /// True when something was dropped.
    fn drop_untrusted(file: &mut SyncFile, path: &Path, collection: &str) -> bool {
        if file.pending_clears.is_empty() {
            return false;
        }
        let key = pending_seal::existing_key();
        let before = file.pending_clears.len();
        file.pending_clears.retain(|pending| {
            let legacy = pending.mac.is_empty()
                && pending.repo_id.is_empty()
                && pending.collection == collection;
            legacy
                || key
                    .as_ref()
                    .is_some_and(|key| pending_seal::verify(key, pending, path))
        });
        let dropped = before - file.pending_clears.len();
        if dropped > 0 {
            eprintln!(
                "[davinci-memory] ignored {dropped} pending remote clear(s) in {}: not recorded by this user",
                path.display()
            );
        }
        dropped > 0
    }

    /// Best effort: a file that cannot be written only costs a re-send.
    fn save_sync(sync: &SyncState, path: &Path) {
        let _ = Self::save_sync_checked(sync, path);
    }

    fn save_sync_checked(sync: &SyncState, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(&sync.file).map_err(|error| error.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    }

    /// True while a recent failure keeps the remote out of use.
    pub fn backing_off(&self) -> bool {
        self.lock()
            .backoff_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// The projection to use right now, or `None` for local-only: backend is
    /// local, or the remote failed recently, or its config is refused.
    pub fn acquire(
        &self,
        config: &VectorMemoryConfig,
        repo_id: &str,
    ) -> Option<Arc<dyn VectorProjection>> {
        if config.projection.backend != MemoryProjectionBackend::Qdrant || self.backing_off() {
            return None;
        }
        let mut inner = self.lock();
        if let Some(injected) = &inner.injected {
            return Some(injected.clone());
        }
        let key = cache_key(config, repo_id);
        if let Some((cached_key, client)) = &inner.cached {
            if *cached_key == key {
                return Some(client.clone());
            }
        }
        match QdrantProjection::new(config, repo_id) {
            Ok(client) => {
                let client: Arc<dyn VectorProjection> = Arc::new(client);
                inner.cached = Some((key, client.clone()));
                Some(client)
            }
            Err(error) => {
                drop(inner);
                self.record(config, Err(error));
                None
            }
        }
    }

    /// Note the outcome of a remote call: Ok clears the backoff, a failure
    /// starts one.
    pub fn record(&self, config: &VectorMemoryConfig, outcome: Result<(), ProjectionError>) {
        let backoff = Duration::from_secs(config.projection.offline_backoff_seconds);
        let mut inner = self.lock();
        match outcome {
            Ok(()) => {
                inner.backoff_until = None;
                inner.health = ProjectionHealth {
                    state: ProjectionState::Ready,
                    code: None,
                    detail: String::new(),
                };
            }
            Err(error) => {
                inner.backoff_until = Some(Instant::now() + backoff);
                let (state, code) = match error {
                    ProjectionError::Offline(_) => (ProjectionState::Offline, QDRANT_OFFLINE),
                    ProjectionError::Refused(_) => (ProjectionState::Refused, QDRANT_REFUSED),
                };
                inner.health = ProjectionHealth {
                    state,
                    code: Some(code),
                    detail: error.to_string(),
                };
            }
        }
    }
}

/// Everything a client depends on. The key is hashed so the cache key can be
/// compared without holding a second copy of the secret.
fn cache_key(config: &VectorMemoryConfig, repo_id: &str) -> String {
    let key_digest = config
        .projection
        .api_key
        .as_ref()
        .map(|key| sha256_hex(key.expose().as_bytes()))
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{repo_id}|{key_digest}",
        config.qdrant_url,
        config.projection.collection_prefix,
        config.projection.allow_remote,
        config.projection.batch_size,
        config.request_timeout_seconds,
        config.embedding_model,
        config.embedding_dimensions,
    )
}
