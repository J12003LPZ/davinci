//! Immutable repository facts shared within a turn. No eager submit-path I/O.
use super::{
    repo_intelligence::{RepoIndex, RepoIntelligence},
    workspace_metadata::WorkspaceMetadata,
};
use davinci_agent::decision::request::WorkspaceDirtyState;
use davinci_agent::runtime::cache::digest;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub struct EngineeringSnapshot {
    pub generation: u64,
    pub root: PathBuf,
    pub index: Arc<RepoIndex>,
    pub metadata: WorkspaceMetadata,
    pub identity: String,
    pub workspace_dirty: WorkspaceDirtyState,
    // A missing revision means that subsystem has supplied no authoritative
    // observation in this turn. Never infer Git state from edited UI paths.
    pub build_revision: Option<String>,
    pub lsp_revision: Option<u64>,
    pub transaction: Option<String>,
}

#[derive(Debug, Default)]
struct State {
    generation: u64,
    snapshot: Option<Arc<EngineeringSnapshot>>,
    builds: u64,
    hits: u64,
    stamps: Vec<(PathBuf, Option<FileStamp>)>,
}

#[derive(Debug, Clone, Default)]
pub struct EngineeringSnapshots(Arc<Mutex<State>>);

impl EngineeringSnapshots {
    pub fn invalidate(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.snapshot = None;
    }

    /// Jev and inspectors may use facts already computed by engineering tools.
    /// They never trigger a filesystem scan or wait for one in progress.
    pub fn peek(&self) -> Option<Arc<EngineeringSnapshot>> {
        self.0.try_lock().ok()?.snapshot.clone()
    }

    /// Reuse observed metadata only while its files and containing directories
    /// still match. Consumers retain their existing read authorization and fall
    /// back to their own reads on a miss; this never starts an index refresh.
    pub fn peek_current(&self, root: &Path) -> Option<Arc<EngineeringSnapshot>> {
        let canonical = root.canonicalize().ok()?;
        let mut state = self.0.try_lock().ok()?;
        let snapshot = state.snapshot.as_ref()?;
        if snapshot.root != canonical
            || !state
                .stamps
                .iter()
                .all(|(path, stamp)| &file_stamp(path) == stamp)
        {
            return None;
        }
        let snapshot = snapshot.clone();
        state.hits += 1;
        Some(snapshot)
    }

    pub fn status(&self) -> serde_json::Value {
        let (builds, hits) = self.counters();
        let snapshot = self.peek();
        serde_json::json!({
            "builds": builds, "hits": hits,
            "generation": snapshot.as_ref().map(|s| s.generation),
            "build_revision": snapshot.as_ref().and_then(|s| s.build_revision.as_ref()),
            "lsp_revision": snapshot.as_ref().and_then(|s| s.lsp_revision),
            "transaction": snapshot.as_ref().and_then(|s| s.transaction.as_ref()),
        })
    }

    pub fn counters(&self) -> (u64, u64) {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        (state.builds, state.hits)
    }

    pub fn get_with_usage(
        &self,
        root: &Path,
        repo: &RepoIntelligence,
        changed: &[String],
        force: bool,
        authorize: &impl Fn(&str) -> Result<(), String>,
    ) -> Result<(Arc<EngineeringSnapshot>, bool), String> {
        let canonical = root
            .canonicalize()
            .map_err(|_| "index_unavailable: root missing")?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !force {
            if let Some(snapshot) = &state.snapshot {
                if snapshot.root != canonical {
                    return Err("outside_workspace: root identity changed".into());
                }
                // Read authorization and confinement are checked on every hit.
                // The turn is immutable; writes, external commands, refresh,
                // and a new turn invalidate it before another consumer reads.
                for path in snapshot
                    .index
                    .files
                    .keys()
                    .chain(snapshot.index.metadata.iter())
                {
                    authorize(path)?;
                    repo.validate_changed_path(path)?;
                }
                for path in changed {
                    authorize(path)?;
                    repo.validate_changed_path(path)?;
                }
                if !repo.has_observed_changes()
                    && state
                        .stamps
                        .iter()
                        .all(|(path, stamp)| &file_stamp(path) == stamp)
                {
                    let snapshot = snapshot.clone();
                    state.hits += 1;
                    return Ok((snapshot, true));
                }
            }
        }
        let index = repo.refresh_observed_authorized(changed, force, authorize)?;
        let metadata = WorkspaceMetadata::discover(repo, &index, authorize)?;
        let identity = digest(
            &serde_json::to_vec(&(
                &index.root,
                &index.parser_version,
                &index.config_identity,
                index
                    .files
                    .iter()
                    .map(|(path, file)| (path, &file.content_hash))
                    .collect::<Vec<_>>(),
                &metadata.hashes,
            ))
            .map_err(|e| e.to_string())?,
        );
        let snapshot = Arc::new(EngineeringSnapshot {
            generation: state.generation,
            root: canonical.clone(),
            index,
            metadata,
            identity,
            workspace_dirty: WorkspaceDirtyState::Unknown,
            build_revision: None,
            lsp_revision: None,
            transaction: None,
        });
        state.stamps = snapshot_stamps(&canonical, &snapshot.index);
        state.builds += 1;
        state.snapshot = Some(snapshot.clone());
        Ok((snapshot, false))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: std::time::SystemTime,
    directory: bool,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok()?,
        directory: metadata.is_dir(),
    })
}

fn snapshot_stamps(root: &Path, index: &RepoIndex) -> Vec<(PathBuf, Option<FileStamp>)> {
    let mut paths = std::collections::BTreeSet::new();
    paths.insert(root.to_path_buf());
    for relative in index.files.keys().chain(index.metadata.iter()) {
        let path = root.join(relative);
        paths.insert(path.clone());
        for parent in path
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(root))
        {
            paths.insert(parent.to_path_buf());
            paths.insert(parent.join(".gitignore"));
            paths.insert(parent.join(".ignore"));
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let stamp = file_stamp(&path);
            (path, stamp)
        })
        .collect()
}
