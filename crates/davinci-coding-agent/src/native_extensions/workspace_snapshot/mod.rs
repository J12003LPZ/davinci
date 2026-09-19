//! Bounded, owner-scoped workspace checkpoints.
//!
//! Checkpoints are local metadata records, never Git commits.  A restore is
//! conflict-first: a file is written only when its current post-image is
//! unchanged from the transaction that owned the edit.  The implementation
//! keeps the captured bytes bounded and never follows a symlink while
//! resolving a user supplied path.

mod model;
mod tools;

pub use model::{
    SnapshotDiffEntry, SnapshotEntry, SnapshotEntryKind, WorkspaceCheckpoint,
    WorkspaceCheckpointArgs, WorkspaceDiffArgs, WorkspaceRestoreArgs, WorkspaceSnapshotConfig,
    WorkspaceSnapshotTelemetry,
};
pub use tools::{tool_spec, TOOL_NAMES};

use davinci_agent::{
    is_sensitive_file_path,
    runtime::transactions::{coordinator_for_context, TransactionCoordinator, TransactionOwner},
    PermissionMode, PermissionPolicy, PermissionState, PermissionVerdict, ToolContext, ToolError,
    ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA_VERSION: u32 = 1;
const STORE_DIR: &str = ".davinci-workspace-snapshots";
const JOURNAL_FILE: &str = ".davinci-workspace-restore.json";
const MAX_ID_BYTES: usize = 128;
const MAX_GIT_IDENTITY_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestoreJournal {
    schema_version: u32,
    checkpoint_id: String,
    paths: Vec<String>,
    started_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    root: PathBuf,
    config: WorkspaceSnapshotConfig,
    permissions: Arc<RwLock<Arc<PermissionState>>>,
    cancellation: Arc<RwLock<Option<Arc<AtomicBool>>>>,
    telemetry: Arc<Mutex<WorkspaceSnapshotTelemetry>>,
}

impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        Self::new(Path::new("."), WorkspaceSnapshotConfig::default())
    }
}

impl WorkspaceSnapshot {
    pub fn new(root: &Path, config: WorkspaceSnapshotConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            config: config.bounded(),
            permissions: Arc::new(RwLock::new(Arc::new(PermissionState::new(
                PermissionPolicy::new(PermissionMode::Ask),
            )))),
            cancellation: Arc::new(RwLock::new(None)),
            telemetry: Arc::new(Mutex::new(WorkspaceSnapshotTelemetry::default())),
        }
    }

    pub fn set_permissions(&self, permissions: Arc<PermissionState>) {
        *self
            .permissions
            .write()
            .unwrap_or_else(|error| error.into_inner()) = permissions;
    }

    pub fn set_cancellation(&self, signal: Option<Arc<AtomicBool>>) {
        *self
            .cancellation
            .write()
            .unwrap_or_else(|error| error.into_inner()) = signal;
    }

    pub fn status(&self) -> Value {
        let telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        json!({
            "enabled": self.config.enabled,
            "root": self.root,
            "schemaVersion": SCHEMA_VERSION,
            "store": self.root.join(STORE_DIR),
            "limits": {
                "maxFiles": self.config.max_files,
                "maxFileBytes": self.config.max_file_bytes,
                "maxTotalBytes": self.config.max_total_bytes,
                "maxCheckpoints": self.config.max_checkpoints
            },
            "telemetry": telemetry,
            "mutation": "checkpoint_metadata_only_until_restore"
        })
    }

    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        self.execute_with_context(&self.root, name, args, None)
    }

    pub fn execute_with_context(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<ToolResult, ToolError> {
        if !TOOL_NAMES.contains(&name) {
            return Err(ToolError::Unknown(name.to_string()));
        }
        let started = SystemTime::now();
        self.bump_requests();
        let result = if !self.config.enabled {
            Ok(json!({
                "schemaVersion": SCHEMA_VERSION,
                "enabled": false,
                "partial": true,
                "complete": false,
                "warnings": ["workspace snapshots are disabled in settings"]
            }))
        } else if self.is_cancelled(context) {
            Ok(json!({
                "schemaVersion": SCHEMA_VERSION,
                "partial": true,
                "complete": false,
                "cancelled": true,
                "warnings": ["workspace snapshot request cancelled before capture"]
            }))
        } else {
            match name {
                "workspace_checkpoint" => self.checkpoint(cwd, args, context),
                "workspace_diff" => self.diff(cwd, args, context),
                "workspace_restore" => self.restore(cwd, args, context),
                _ => Err(format!("unknown workspace snapshot tool '{name}'")),
            }
        };
        let latency_ms = started.elapsed().unwrap_or_default().as_secs_f64() * 1000.0;
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.last_latency_ms = latency_ms;
        if result.is_err() {
            telemetry.failures = telemetry.failures.saturating_add(1);
        }
        let telemetry_value = serde_json::to_value(&*telemetry).unwrap_or_else(|_| json!({}));
        drop(telemetry);
        result
            .map(|mut details| {
                details["telemetry"] = telemetry_value;
                details["telemetry"]["latencyMs"] = json!(latency_ms);
                ToolResult {
                    content: serde_json::to_string_pretty(&details)
                        .unwrap_or_else(|_| details.to_string()),
                    details: Some(details),
                    is_error: false,
                }
            })
            .map_err(ToolError::Failed)
    }

    fn checkpoint(
        &self,
        cwd: &Path,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<Value, String> {
        let request: WorkspaceCheckpointArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("workspace_checkpoint: invalid arguments: {error}"))?;
        let paths = self.request_paths(&request.paths, request.path.as_deref())?;
        self.authorize("workspace_checkpoint", args, cwd, None)?;
        let transaction_id =
            self.validate_transaction(cwd, request.transaction_id.as_deref(), context)?;
        let entries = self.capture_paths(cwd, &paths, context)?;
        let identity = self.workspace_identity(cwd)?;
        let id = digest(
            &serde_json::to_vec(&(&identity, &entries, request.label.as_deref()))
                .map_err(|error| error.to_string())?,
        );
        let checkpoint = WorkspaceCheckpoint {
            schema_version: SCHEMA_VERSION,
            id: format!("ws-{}", &id[..24]),
            workspace: cwd.to_string_lossy().to_string(),
            workspace_identity: identity,
            entries,
            transaction_id,
            label: request.label.filter(|value| !value.trim().is_empty()),
            created_at_ms: now_ms(),
        };
        self.save_record(cwd, &checkpoint)?;
        self.bump_checkpoints();
        self.prune_records(cwd)?;
        Ok(json!({
            "schemaVersion": SCHEMA_VERSION,
            "checkpoint": self.public_checkpoint(&checkpoint),
            "partial": false,
            "complete": true
        }))
    }

    fn diff(
        &self,
        cwd: &Path,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<Value, String> {
        let request: WorkspaceDiffArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("workspace_diff: invalid arguments: {error}"))?;
        let checkpoint = self.load_record(cwd, &request.checkpoint_id)?;
        self.authorize("workspace_diff", args, cwd, None)?;
        self.verify_checkpoint_workspace(cwd, &checkpoint)?;
        let selected = self.select_entries(&checkpoint, &request.paths)?;
        let paths = selected
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let current = self.capture_paths(cwd, &paths, context)?;
        let mut changes = Vec::new();
        for expected in selected {
            let actual = current
                .iter()
                .find(|entry| entry.path == expected.path)
                .ok_or_else(|| format!("current capture omitted {}", expected.path))?;
            changes.push(compare_entry(expected, actual));
        }
        self.bump_diffs();
        Ok(json!({
            "schemaVersion": SCHEMA_VERSION,
            "checkpointId": checkpoint.id,
            "changes": changes,
            "partial": false,
            "complete": true
        }))
    }

    fn restore(
        &self,
        cwd: &Path,
        args: &Value,
        context: Option<&ToolContext>,
    ) -> Result<Value, String> {
        let request: WorkspaceRestoreArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("workspace_restore: invalid arguments: {error}"))?;
        let checkpoint = self.load_record(cwd, &request.checkpoint_id)?;
        self.authorize("workspace_restore", args, cwd, None)?;
        self.verify_checkpoint_workspace(cwd, &checkpoint)?;
        for entry in &checkpoint.entries {
            self.authorize("workspace_restore", args, cwd, Some(&entry.path))?;
        }
        let transaction =
            self.transaction_summary(cwd, request.transaction_id.as_deref(), context)?;
        let paths = checkpoint
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let current = self.capture_paths(cwd, &paths, context)?;
        let mut changes = Vec::new();
        let mut conflicts = Vec::new();
        for expected in &checkpoint.entries {
            let actual = current
                .iter()
                .find(|entry| entry.path == expected.path)
                .ok_or_else(|| format!("current capture omitted {}", expected.path))?;
            let mut change = compare_entry(expected, actual);
            change.safe_to_restore = safe_restore(actual, expected, transaction.as_ref());
            if !change.safe_to_restore
                && change.status != "already_restored"
                && change.status != "unchanged"
            {
                conflicts.push(json!({
                    "path": change.path,
                    "reason": "current postimage is not owned by the supplied transaction",
                    "currentHash": change.current_hash,
                    "checkpointHash": change.checkpoint_hash
                }));
            }
            changes.push(change);
        }
        if !conflicts.is_empty() {
            self.bump_conflicts();
            return Ok(json!({
                "schemaVersion": SCHEMA_VERSION,
                "checkpointId": checkpoint.id,
                "changes": changes,
                "conflicts": conflicts,
                "partial": true,
                "complete": false,
                "mutation": "none"
            }));
        }
        if self.is_cancelled(context) {
            return Ok(json!({
                "schemaVersion": SCHEMA_VERSION,
                "checkpointId": checkpoint.id,
                "changes": changes,
                "partial": true,
                "complete": false,
                "cancelled": true,
                "mutation": "none"
            }));
        }
        let restore_paths = changes
            .iter()
            .filter(|change| change.status != "already_restored" && change.status != "unchanged")
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        if restore_paths.is_empty() {
            self.bump_restores();
            return Ok(json!({
                "schemaVersion": SCHEMA_VERSION,
                "checkpointId": checkpoint.id,
                "changes": changes,
                "partial": false,
                "complete": true,
                "mutation": "none"
            }));
        }
        self.write_journal(cwd, &checkpoint.id, &restore_paths)?;
        for entry in &checkpoint.entries {
            let change = changes
                .iter()
                .find(|change| change.path == entry.path)
                .expect("change created for every checkpoint entry");
            if change.status != "already_restored" && change.status != "unchanged" {
                self.apply_entry(cwd, entry)?;
            }
        }
        let verified = self.capture_paths(cwd, &restore_paths, context)?;
        for path in &restore_paths {
            let expected = checkpoint
                .entries
                .iter()
                .find(|entry| &entry.path == path)
                .ok_or_else(|| format!("checkpoint entry disappeared for {path}"))?;
            let actual = verified
                .iter()
                .find(|entry| &entry.path == path)
                .ok_or_else(|| format!("restore verification omitted {path}"))?;
            if compare_entry(expected, actual).status != "unchanged" {
                return Err(format!("restore verification failed for {path}"));
            }
        }
        self.remove_journal(cwd)?;
        self.bump_restores();
        Ok(json!({
            "schemaVersion": SCHEMA_VERSION,
            "checkpointId": checkpoint.id,
            "changes": changes,
            "partial": false,
            "complete": true,
            "mutation": "restored",
            "verifiedPaths": restore_paths
        }))
    }

    fn request_paths(&self, paths: &[String], path: Option<&str>) -> Result<Vec<String>, String> {
        let mut values = paths.to_vec();
        if let Some(path) = path {
            values.push(path.to_string());
        }
        if values.is_empty() {
            return Err("at least one workspace path is required".into());
        }
        if values.len() > self.config.max_files {
            return Err(format!(
                "at most {} workspace paths are supported",
                self.config.max_files
            ));
        }
        let mut normalized = Vec::new();
        for value in values {
            let path = normalize_path(&value)?;
            if !normalized.contains(&path) {
                normalized.push(path);
            }
        }
        Ok(normalized)
    }

    fn select_entries<'a>(
        &self,
        checkpoint: &'a WorkspaceCheckpoint,
        paths: &[String],
    ) -> Result<Vec<&'a SnapshotEntry>, String> {
        if paths.is_empty() {
            return Ok(checkpoint.entries.iter().collect());
        }
        let requested = paths
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Result<Vec<_>, _>>()?;
        let mut selected = Vec::new();
        for path in requested {
            let entry = checkpoint
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .ok_or_else(|| format!("path '{path}' is not part of checkpoint"))?;
            selected.push(entry);
        }
        Ok(selected)
    }

    fn capture_paths(
        &self,
        cwd: &Path,
        paths: &[String],
        context: Option<&ToolContext>,
    ) -> Result<Vec<SnapshotEntry>, String> {
        let mut total = 0usize;
        let mut entries = Vec::with_capacity(paths.len());
        for path in paths {
            if self.is_cancelled(context) {
                return Err("workspace snapshot capture cancelled".into());
            }
            entries.push(capture_entry(
                cwd,
                path,
                self.config.max_file_bytes,
                self.config.max_total_bytes,
                &mut total,
            )?);
        }
        Ok(entries)
    }

    fn authorize(
        &self,
        tool: &str,
        args: &Value,
        cwd: &Path,
        path: Option<&str>,
    ) -> Result<(), String> {
        let state = self
            .permissions
            .read()
            .map_err(|_| "permission state unavailable".to_string())?
            .clone();
        let policy = state
            .lock()
            .map_err(|_| "permission policy unavailable".to_string())?;
        let input = path.map_or_else(|| args.clone(), |path| json!({ "path": path }));
        match policy.decide("workspace-snapshot", tool, &input, cwd) {
            PermissionVerdict::Allow => Ok(()),
            _ => Err(format!("{tool} denied by current permissions")),
        }
    }

    fn validate_transaction(
        &self,
        cwd: &Path,
        transaction_id: Option<&str>,
        context: Option<&ToolContext>,
    ) -> Result<Option<String>, String> {
        let Some(id) = transaction_id else {
            return Ok(None);
        };
        if id.is_empty() || id.len() > MAX_ID_BYTES || id.chars().any(char::is_control) {
            return Err("invalid transactionId".into());
        }
        Ok(Some(
            self.transaction_summary(cwd, Some(id), context)?
                .map(|summary| summary.id)
                .unwrap_or_else(|| id.to_string()),
        ))
    }

    fn transaction_summary(
        &self,
        cwd: &Path,
        transaction_id: Option<&str>,
        context: Option<&ToolContext>,
    ) -> Result<Option<davinci_agent::runtime::transactions::TransactionSummary>, String> {
        let Some(id) = transaction_id else {
            return Ok(None);
        };
        let coordinator = if let Some(context) = context {
            coordinator_for_context(cwd, context).map_err(|error| error.to_string())?
        } else {
            TransactionCoordinator::new(&self.root, TransactionOwner::default())?
        };
        coordinator
            .status(id)
            .map(Some)
            .map_err(|error| format!("transactionId is unavailable or not owned: {error}"))
    }

    fn workspace_identity(&self, cwd: &Path) -> Result<String, String> {
        let canonical =
            fs::canonicalize(cwd).map_err(|error| format!("workspace root: {error}"))?;
        let mut material = Vec::new();
        append_identity_field(
            &mut material,
            "worktree",
            canonical.to_string_lossy().as_bytes(),
        );
        if let Some(git_dir) = git_dir_for_workspace(cwd) {
            let common_dir = git_common_dir(&git_dir);
            append_identity_path(&mut material, "git-dir", &git_dir);
            append_identity_path(&mut material, "git-common-dir", &common_dir);
            append_identity_file(&mut material, "git-head", &git_dir.join("HEAD"));
            append_identity_file(&mut material, "git-common-head", &common_dir.join("HEAD"));
            append_identity_file(&mut material, "git-index", &git_dir.join("index"));
            append_identity_file(&mut material, "git-common-index", &common_dir.join("index"));
        }
        Ok(digest(&material))
    }

    fn verify_checkpoint_workspace(
        &self,
        cwd: &Path,
        checkpoint: &WorkspaceCheckpoint,
    ) -> Result<(), String> {
        let current = self.workspace_identity(cwd)?;
        if current != checkpoint.workspace_identity {
            return Err(format!(
                "checkpoint '{}' belongs to a different workspace",
                checkpoint.id
            ));
        }
        Ok(())
    }

    fn store_dir(&self, cwd: &Path) -> PathBuf {
        cwd.join(STORE_DIR)
    }

    fn save_record(&self, cwd: &Path, checkpoint: &WorkspaceCheckpoint) -> Result<(), String> {
        fs::create_dir_all(self.store_dir(cwd))
            .map_err(|error| format!("create checkpoint store: {error}"))?;
        let bytes = serde_json::to_vec_pretty(checkpoint).map_err(|error| error.to_string())?;
        atomic_write(
            &self.store_dir(cwd).join(format!("{}.json", checkpoint.id)),
            &bytes,
        )
    }

    fn load_record(&self, cwd: &Path, id: &str) -> Result<WorkspaceCheckpoint, String> {
        if id.is_empty()
            || id.len() > MAX_ID_BYTES
            || id
                .chars()
                .any(|character| character.is_control() || character == '\\' || character == '/')
        {
            return Err("invalid checkpointId".into());
        }
        let bytes = fs::read(self.store_dir(cwd).join(format!("{id}.json")))
            .map_err(|error| format!("checkpoint '{id}' unavailable: {error}"))?;
        if bytes.len() > self.config.max_total_bytes.saturating_mul(2) {
            return Err("checkpoint record exceeds the configured bound".into());
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid checkpoint record: {error}"))
    }

    fn prune_records(&self, cwd: &Path) -> Result<(), String> {
        let mut records = fs::read_dir(self.store_dir(cwd))
            .map_err(|error| format!("read checkpoint store: {error}"))?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        records.sort_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH)
        });
        while records.len() > self.config.max_checkpoints {
            let entry = records.remove(0);
            let _ = fs::remove_file(entry.path());
        }
        Ok(())
    }

    fn write_journal(
        &self,
        cwd: &Path,
        checkpoint_id: &str,
        paths: &[String],
    ) -> Result<(), String> {
        let journal = RestoreJournal {
            schema_version: SCHEMA_VERSION,
            checkpoint_id: checkpoint_id.to_string(),
            paths: paths.to_vec(),
            started_at_ms: now_ms(),
        };
        let bytes = serde_json::to_vec(&journal).map_err(|error| error.to_string())?;
        atomic_write(&cwd.join(JOURNAL_FILE), &bytes)
    }

    fn remove_journal(&self, cwd: &Path) -> Result<(), String> {
        match fs::remove_file(cwd.join(JOURNAL_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("remove restore journal: {error}")),
        }
    }

    fn apply_entry(&self, cwd: &Path, entry: &SnapshotEntry) -> Result<(), String> {
        let target = safe_join(cwd, &entry.path)?;
        if target == cwd.join(STORE_DIR) || target == cwd.join(JOURNAL_FILE) {
            return Err("checkpoint metadata cannot be restored".into());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create restore parent: {error}"))?;
        }
        let temp = target.with_extension(format!("davinci-restore-{}", std::process::id()));
        let _ = fs::remove_file(&temp);
        match &entry.kind {
            SnapshotEntryKind::Missing => {
                remove_path(&target)?;
            }
            SnapshotEntryKind::File => {
                let bytes = entry
                    .bytes
                    .as_deref()
                    .ok_or_else(|| format!("checkpoint file bytes missing for {}", entry.path))?;
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)
                    .map_err(|error| format!("create restore file: {error}"))?;
                file.write_all(bytes)
                    .map_err(|error| format!("write restore file: {error}"))?;
                file.sync_all()
                    .map_err(|error| format!("sync restore file: {error}"))?;
                remove_path(&target)?;
                fs::rename(&temp, &target)
                    .map_err(|error| format!("publish restore file: {error}"))?;
                set_mode(&target, entry.mode);
            }
            SnapshotEntryKind::Symlink { target: link } => {
                validate_symlink_target(cwd, &target, link)?;
                remove_path(&target)?;
                create_symlink(link, &target)?;
            }
        }
        Ok(())
    }

    fn public_checkpoint(&self, checkpoint: &WorkspaceCheckpoint) -> Value {
        let mut value = serde_json::to_value(checkpoint).unwrap_or_else(|_| json!({}));
        if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
            for entry in entries {
                if let Some(object) = entry.as_object_mut() {
                    object.remove("bytes");
                }
            }
        }
        value
    }

    fn is_cancelled(&self, context: Option<&ToolContext>) -> bool {
        context.is_some_and(ToolContext::is_aborted)
            || self
                .cancellation
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .is_some_and(|signal| signal.load(Ordering::Acquire))
    }

    fn bump_requests(&self) {
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.requests = telemetry.requests.saturating_add(1);
    }

    fn bump_checkpoints(&self) {
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.checkpoints = telemetry.checkpoints.saturating_add(1);
    }

    fn bump_diffs(&self) {
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.diffs = telemetry.diffs.saturating_add(1);
    }

    fn bump_restores(&self) {
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.restores = telemetry.restores.saturating_add(1);
    }

    fn bump_conflicts(&self) {
        let mut telemetry = self
            .telemetry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        telemetry.conflicts = telemetry.conflicts.saturating_add(1);
    }
}

fn normalize_path(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("invalid workspace path".into());
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!(
            "workspace path '{value}' must be relative and contained"
        ));
    }
    let normalized = value.replace('\\', "/");
    let trimmed = normalized.trim_matches('/').to_string();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".git"
        || trimmed.starts_with(".git/")
        || trimmed == STORE_DIR
        || trimmed.starts_with(&format!("{STORE_DIR}/"))
        || trimmed == JOURNAL_FILE
        || is_sensitive_file_path(&trimmed)
    {
        return Err(format!(
            "workspace path '{value}' is not eligible for snapshots"
        ));
    }
    Ok(trimmed)
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let normalized = normalize_path(relative)?;
    let path = root.join(&normalized);
    let components = Path::new(&normalized).components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        if index + 1 < components.len()
            && fs::symlink_metadata(&current)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
        {
            return Err(format!(
                "workspace path '{relative}' traverses a symlinked parent"
            ));
        }
    }
    if let (Ok(root_canonical), Ok(path_canonical)) =
        (fs::canonicalize(root), fs::canonicalize(&path))
    {
        if !path_canonical.starts_with(root_canonical) {
            return Err(format!(
                "workspace path '{relative}' resolves outside the workspace"
            ));
        }
    }
    Ok(path)
}

fn capture_entry(
    root: &Path,
    relative: &str,
    max_file_bytes: usize,
    max_total_bytes: usize,
    total: &mut usize,
) -> Result<SnapshotEntry, String> {
    let path = safe_join(root, relative)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SnapshotEntry {
                path: relative.to_string(),
                kind: SnapshotEntryKind::Missing,
                hash: None,
                bytes: None,
                mode: None,
                size: 0,
            })
        }
        Err(error) => return Err(format!("snapshot metadata for {relative}: {error}")),
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(&path)
            .map_err(|error| format!("read symlink target for {relative}: {error}"))?
            .to_string_lossy()
            .to_string();
        validate_symlink_target(root, &path, &target)?;
        return Ok(SnapshotEntry {
            path: relative.to_string(),
            kind: SnapshotEntryKind::Symlink {
                target: target.clone(),
            },
            hash: Some(digest(target.as_bytes())),
            bytes: None,
            mode: None,
            size: target.len() as u64,
        });
    }
    if !file_type.is_file() {
        return Err(format!(
            "workspace path '{relative}' is not a regular file or symlink"
        ));
    }
    let size = metadata.len();
    if size > max_file_bytes as u64 {
        return Err(format!(
            "workspace file '{relative}' exceeds the file bound"
        ));
    }
    let size_usize =
        usize::try_from(size).map_err(|_| "workspace file size overflow".to_string())?;
    if total.saturating_add(size_usize) > max_total_bytes {
        return Err("workspace checkpoint exceeds the total byte bound".into());
    }
    let mut bytes = Vec::with_capacity(size_usize);
    File::open(&path)
        .map_err(|error| format!("open workspace file {relative}: {error}"))?
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read workspace file {relative}: {error}"))?;
    *total = total.saturating_add(bytes.len());
    let hash = digest(&bytes);
    Ok(SnapshotEntry {
        path: relative.to_string(),
        kind: SnapshotEntryKind::File,
        hash: Some(hash),
        bytes: Some(bytes),
        mode: file_mode(&metadata),
        size,
    })
}

fn compare_entry(expected: &SnapshotEntry, actual: &SnapshotEntry) -> SnapshotDiffEntry {
    let same = expected.kind == actual.kind
        && expected.hash == actual.hash
        && expected.size == actual.size
        && expected.mode == actual.mode;
    SnapshotDiffEntry {
        path: expected.path.clone(),
        status: if same { "unchanged" } else { "changed" }.to_string(),
        checkpoint_hash: expected.hash.clone(),
        current_hash: actual.hash.clone(),
        safe_to_restore: same,
    }
}

fn safe_restore(
    actual: &SnapshotEntry,
    expected: &SnapshotEntry,
    transaction: Option<&davinci_agent::runtime::transactions::TransactionSummary>,
) -> bool {
    if actual.kind == expected.kind
        && actual.hash == expected.hash
        && actual.size == expected.size
        && actual.mode == expected.mode
    {
        return true;
    }
    let Some(transaction) = transaction else {
        return false;
    };
    transaction
        .applied_hashes
        .get(&actual.path)
        .or_else(|| transaction.proposed_hashes.get(&actual.path))
        .is_some_and(|hash| hash.as_deref() == actual.hash.as_deref())
}

fn remove_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::remove_dir(path).map_err(|error| format!("remove restore directory: {error}"))
        }
        Ok(_) => fs::remove_file(path).map_err(|error| format!("remove restore path: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect restore path: {error}")),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temp);
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(|error| format!("create atomic file: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("write atomic file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync atomic file: {error}"))?;
    }
    let _ = fs::remove_file(path);
    fs::rename(&temp, path).map_err(|error| format!("publish atomic file: {error}"))
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.mode())
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

fn set_mode(path: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

fn create_symlink(target: &str, path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, path)
            .map_err(|error| format!("create restore symlink: {error}"))
    }
    #[cfg(windows)]
    {
        let resolved = path
            .parent()
            .map(|parent| parent.join(target))
            .unwrap_or_else(|| PathBuf::from(target));
        if resolved.is_dir() {
            std::os::windows::fs::symlink_dir(target, path)
        } else {
            std::os::windows::fs::symlink_file(target, path)
        }
        .map_err(|error| format!("create restore symlink: {error}"))
    }
}

fn validate_symlink_target(root: &Path, link_path: &Path, target: &str) -> Result<(), String> {
    let target_path = Path::new(target);
    if target.is_empty()
        || target_path.is_absolute()
        || target_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(format!(
            "symlink target for '{}' is outside the workspace",
            link_path.display()
        ));
    }
    if let Ok(root_canonical) = fs::canonicalize(root) {
        let candidate = link_path.parent().unwrap_or(root).join(target_path);
        if let Ok(candidate_canonical) = fs::canonicalize(candidate) {
            if !candidate_canonical.starts_with(root_canonical) {
                return Err(format!(
                    "symlink target for '{}' resolves outside the workspace",
                    link_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn git_dir_for_workspace(cwd: &Path) -> Option<PathBuf> {
    let dot_git = cwd.join(".git");
    let metadata = fs::symlink_metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return fs::canonicalize(dot_git).ok();
    }
    if !metadata.is_file() {
        return None;
    }
    let contents = fs::read_to_string(&dot_git).ok()?;
    let value = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        dot_git.parent()?.join(path)
    };
    fs::canonicalize(resolved).ok()
}

fn git_common_dir(git_dir: &Path) -> PathBuf {
    let commondir = git_dir.join("commondir");
    let Ok(contents) = fs::read_to_string(&commondir) else {
        return git_dir.to_path_buf();
    };
    let value = contents.trim();
    if value.is_empty() {
        return git_dir.to_path_buf();
    }
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        git_dir.join(path)
    };
    fs::canonicalize(resolved).unwrap_or_else(|_| git_dir.to_path_buf())
}

fn append_identity_field(material: &mut Vec<u8>, label: &str, value: &[u8]) {
    material.extend_from_slice(label.as_bytes());
    material.push(0);
    material.extend_from_slice(value);
    material.push(0);
}

fn append_identity_path(material: &mut Vec<u8>, label: &str, path: &Path) {
    let value = fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    append_identity_field(material, label, value.as_bytes());
}

fn append_identity_file(material: &mut Vec<u8>, label: &str, path: &Path) {
    let mut value = Vec::new();
    if let Ok(file) = File::open(path) {
        let mut bytes = Vec::new();
        let limit = u64::try_from(MAX_GIT_IDENTITY_FILE_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if file.take(limit).read_to_end(&mut bytes).is_ok() {
            let truncated = bytes.len() > MAX_GIT_IDENTITY_FILE_BYTES;
            if truncated {
                bytes.truncate(MAX_GIT_IDENTITY_FILE_BYTES);
            }
            value.extend_from_slice(digest(&bytes).as_bytes());
            value.extend_from_slice(
                format!(
                    ":{}:{}",
                    bytes.len(),
                    if truncated { "truncated" } else { "complete" }
                )
                .as_bytes(),
            );
        }
    }
    if value.is_empty() {
        value.extend_from_slice(b"missing");
    }
    append_identity_field(material, label, &value);
}
