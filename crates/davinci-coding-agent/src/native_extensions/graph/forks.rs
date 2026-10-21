//! Fork publication owns a separate workspace; parent evidence is never rebound.
use super::mutation::MutationBaseline;
use super::operations::{reset_execution_cursor, ForkPreview};
use super::types::{GraphLifecycle, GraphRun, Phase, Role, TaskStatus};
use super::{blobs, continuation, git, replay, store, workspace_files};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(super) fn workspace_path(cwd: &Path, id: &str) -> PathBuf {
    cwd.join(store::CONFIG_DIR)
        .join("graph/forks")
        .join(id)
        .join("workspace")
}

pub(super) fn published_workspace(cwd: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    if !store::is_safe_run_id(id) {
        return Err("invalid fork identity".into());
    }
    let workspace = workspace_path(cwd, id);
    if !workspace.exists() {
        return Ok(None);
    }
    // Check the workspace itself as an ancestor, too.
    let relative = workspace
        .strip_prefix(cwd)
        .map_err(|error| error.to_string())?
        .join("state-probe");
    workspace_files::checked_path(cwd, &relative).map_err(|error| error.to_string())?;
    if !store::run_dir(&workspace, id).join("state.json").exists() {
        return Ok(None);
    }
    store::load_run_checked(&workspace, id)?;
    Ok(Some(workspace))
}

fn selected_boundary(
    parent: &GraphRun,
    preview: &ForkPreview,
    cwd: &Path,
) -> Result<(MutationBaseline, super::continuation::GraphContinuation), String> {
    let cursor = parent
        .continuation
        .as_ref()
        .ok_or("Fork requires a durable execution cursor")?;
    let first_writer = parent
        .tasks
        .iter()
        .filter(|task| {
            task.role == Role::Writer
                && task.attempts > 0
                && preview.invalidated_node_ids.contains(&task.id)
        })
        .min_by_key(|task| task.started_at.unwrap_or(u64::MAX));
    if let Some(writer) = first_writer {
        if let Some(baseline) = cursor.saved_attempt_baselines.get(&writer.id) {
            return Ok((baseline.clone(), cursor.clone()));
        }
        // A historical in-progress cursor retains the preceding completed
        // milestone; cloning the final cursor would lose that boundary.
        for historic in super::history::checkpoint_runs(cwd, &parent.run_id)?
            .into_iter()
            .rev()
        {
            if let Some(previous) = historic.continuation {
                if let Some(delivery) = &previous.delivery {
                    if delivery.writer_task.as_deref() == Some(&writer.id) {
                        if let Some(baseline) = &delivery.attempt_baseline {
                            return Ok((baseline.clone(), previous));
                        }
                    }
                }
            }
        }
        for delivery in [&cursor.delivery, &cursor.completed_delivery]
            .into_iter()
            .flatten()
        {
            if delivery.writer_task.as_deref() == Some(&writer.id) {
                return delivery
                    .attempt_baseline
                    .clone()
                    .map(|baseline| (baseline, cursor.clone()))
                    .ok_or_else(|| {
                        format!("Fork checkpoint for '{}' has no exact baseline", writer.id)
                    });
            }
        }
        return Err(format!(
            "Fork checkpoint for '{}' is unavailable; parent preserved",
            writer.id
        ));
    }
    Ok((super::mutation::capture_baseline(cwd)?, cursor.clone()))
}

// Baselines deliberately omit links. Until their historical identities are
// recorded, never publish a tree silently missing a tracked link/submodule.
fn validate_tree_entries(cwd: &Path) -> Result<(), String> {
    if cwd.join(".git").exists() {
        let index = git::run(cwd, &["ls-files", "--stage", "-z"])?;
        let head = git::run(cwd, &["ls-tree", "-r", "-z", "HEAD"])?;
        for output in [&index, &head] {
            if output
                .split(|byte| *byte == 0)
                .any(|entry| entry.starts_with(b"120000 ") || entry.starts_with(b"160000 "))
            {
                return Err("Fork checkpoint cannot restore historical symlink/submodule identity; parent preserved".into());
            }
        }
    }
    fn walk(dir: &Path) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if matches!(
                entry.file_name().to_str(),
                Some(".git" | ".davinci" | ".pi" | "target" | "node_modules")
            ) {
                continue;
            }
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                return Err(
                    "Fork checkpoint cannot restore historical symlink identity; parent preserved"
                        .into(),
                );
            }
            if kind.is_dir() {
                walk(&entry.path())?;
            }
        }
        Ok(())
    }
    walk(cwd)
}

fn checkpoint_bytes(
    cwd: &Path,
    baseline: &MutationBaseline,
    path: &str,
) -> Result<Vec<u8>, String> {
    let fingerprint = &baseline.files[path];
    let bytes = baseline
        .old_bytes(path, &blobs::dir(cwd))
        .or_else(|| workspace_files::read(cwd, Path::new(path)).ok())
        .ok_or_else(|| format!("Fork checkpoint bytes unavailable for '{path}'"))?;
    if bytes.len() as u64 != fingerprint.len || replay::sha256_hex(&bytes) != fingerprint.hash {
        return Err(format!(
            "Fork checkpoint bytes unavailable or changed for '{path}'; parent preserved"
        ));
    }
    Ok(bytes)
}

fn checked_relative(path: &str) -> Result<(), String> {
    let mut components = Path::new(path).components();
    let first = components.next().ok_or("empty fork checkpoint path")?;
    if !matches!(first, Component::Normal(_))
        || first.as_os_str() == ".git"
        || path.starts_with(".davinci/graph/")
        || path.starts_with(".pi/graph/")
        || components.any(|component| {
            !matches!(component, Component::Normal(_)) || component.as_os_str() == ".git"
        })
    {
        return Err(format!("Unsupported fork checkpoint path '{path}'"));
    }
    Ok(())
}

pub(super) fn execute(
    parent: &GraphRun,
    preview: &ForkPreview,
    cwd: &Path,
) -> Result<GraphRun, String> {
    if preview.parent_run_id != parent.run_id
        || preview.new_run_id == parent.run_id
        || !store::is_safe_run_id(&preview.new_run_id)
    {
        return Err("Fork preview identity differs from its parent".into());
    }
    // Verify host-derived selection rather than trusting serialized preview node IDs.
    let expected = super::operations::generate_fork_preview(
        parent,
        &preview.fork_node_id,
        preview.strategy,
        Some(cwd),
    )?;
    if expected.invalidated_node_ids != preview.invalidated_node_ids
        || expected.preserved_node_ids != preview.preserved_node_ids
        || expected.checkpoint_ref != preview.checkpoint_ref
    {
        return Err("Fork preview selection is stale".into());
    }
    let mut resumable_parent = parent.clone();
    resumable_parent.phase = Phase::Blocked; // Completed runs may be fork sources.
    continuation::validate_resume(&resumable_parent, cwd)?;
    validate_tree_entries(cwd)?;
    let (baseline, boundary_cursor) = selected_boundary(parent, preview, cwd)?;
    let first_invalidated_writer = parent
        .tasks
        .iter()
        .filter(|task| {
            task.role == Role::Writer
                && task.attempts > 0
                && preview.invalidated_node_ids.contains(&task.id)
        })
        .min_by_key(|task| task.started_at.unwrap_or(u64::MAX));
    if let Some(boundary_writer) = first_invalidated_writer {
        for task in parent.tasks.iter().filter(|task| {
            task.role == Role::Writer
                && task.status == TaskStatus::Succeeded
                && preview.preserved_node_ids.contains(&task.id)
        }) {
            if task
                .mutation
                .as_ref()
                .is_some_and(|mutation| !mutation.files.is_empty())
                && !super::operations::affected_nodes(parent, &task.id)?
                    .contains(&boundary_writer.id)
            {
                return Err(format!("Independent writer '{}' has no proven place before the selected workspace checkpoint; parent preserved", task.id));
            }
        }
    }
    // Preflight exact bytes before creating a worktree or pinning the parent.
    for path in baseline.files.keys() {
        checked_relative(path)?;
        checkpoint_bytes(cwd, &baseline, path)?;
    }
    let workspace = workspace_path(cwd, &preview.new_run_id);
    let container = workspace.parent().unwrap();
    // Do not let an existing link redirect worktree creation outside the parent.
    let mut directory = cwd.to_path_buf();
    for name in [store::CONFIG_DIR, "graph", "forks"] {
        directory.push(name);
        match std::fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let metadata = std::fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Fork destination ancestor is not an ordinary directory".into());
        }
    }
    std::fs::create_dir(container)
        .map_err(|error| format!("Fork destination must be new: {error}"))?;
    let is_git = cwd.join(".git").exists();
    let result = (|| -> Result<GraphRun, String> {
        if is_git {
            let path = workspace.to_str().ok_or("non-UTF-8 fork workspace")?;
            git::run(
                cwd,
                &["worktree", "add", "--detach", "--no-checkout", path, "HEAD"],
            )?;
        } else {
            std::fs::create_dir(&workspace).map_err(|error| error.to_string())?;
        }
        for path in baseline.files.keys() {
            let bytes = checkpoint_bytes(cwd, &baseline, path)?;
            let destination = workspace.join(path);
            std::fs::create_dir_all(destination.parent().unwrap())
                .map_err(|error| error.to_string())?;
            std::fs::write(&destination, bytes).map_err(|error| error.to_string())?;
            // Preserve executable permission for unchanged regular source files.
            if let Ok(metadata) = std::fs::symlink_metadata(cwd.join(path)) {
                if metadata.is_file() && !metadata.file_type().is_symlink() {
                    std::fs::set_permissions(&destination, metadata.permissions())
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        let mut child = parent.clone();
        child.continuation = Some(boundary_cursor.clone());
        let affected: BTreeSet<_> = preview.invalidated_node_ids.iter().cloned().collect();
        reset_execution_cursor(&mut child, &affected)?;
        child.run_id = preview.new_run_id.clone();
        child.cwd = workspace.to_string_lossy().into_owned();
        child.revision = 1;
        child.checkpoint_sequence = 0;
        child.phase = Phase::Classify;
        child.lifecycle = Some(GraphLifecycle::Stopped);
        child.blocked_reason = None;
        child.control_history.clear();
        child.verification = None;
        child.verification_bundle = None;
        child.review_coverage = None;
        child.resource_snapshot = None;
        if parent.budgets.run_deadline_ms > 0 {
            child.budgets.run_deadline_ms = expected
                .remaining_ceiling_ms
                .filter(|remaining| *remaining > 0)
                .ok_or("Parent deadline exhausted; fork cannot renew its budget")?;
        }
        child.counters.started_at = store::now_ms();
        // Parent worker sessions and attempt bindings remain immutable in the
        // pinned ancestor. Inherited artifacts are data, never child executions.
        child.continuation.as_mut().unwrap().attempt_history.clear();
        child
            .continuation
            .as_mut()
            .unwrap()
            .saved_attempt_baselines
            .retain(|id, _| !affected.contains(id));
        let parent_blob_dir = blobs::dir(cwd);
        let child_blob_dir = blobs::dir(&workspace);
        for hash in child.baseline_hashes() {
            if let Some(bytes) = blobs::get(&parent_blob_dir, &hash) {
                if replay::sha256_hex(&bytes) != hash {
                    return Err("Corrupt parent baseline blob".into());
                }
                blobs::put(&child_blob_dir, &hash, &bytes).map_err(|error| error.to_string())?;
            }
        }
        for task in &mut child.tasks {
            task.attempts = 0;
            if affected.contains(&task.id) {
                task.status = TaskStatus::Pending;
                task.artifact_file = None;
                task.error = None;
                task.mutation = None;
                task.fingerprint = None;
                task.started_at = None;
                task.ended_at = None;
                task.last_activity = None;
                task.last_message = None;
            } else if task.status == TaskStatus::Succeeded && task.artifact_file.is_some() {
                let artifact = store::read_artifact(cwd, &parent.run_id, &task.id, task.expect)
                    .map_err(|errors| errors.join("; "))?;
                store::write_artifact(
                    &store::artifact_path(&workspace, &child.run_id, &task.id),
                    &artifact,
                )
                .map_err(|error| error.to_string())?;
                if let Some(fingerprint) = &task.fingerprint {
                    store::write_task_fingerprint(&workspace, &child.run_id, &task.id, fingerprint)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        continuation::validate_resume(&child, &workspace)?;
        store::record_ancestor_run(&workspace, &child.run_id, &parent.run_id)
            .map_err(|error| error.to_string())?;
        // Pin only after materialization is validated; publish the child last.
        store::pin_run(cwd, &parent.run_id).map_err(|error| error.to_string())?;
        let entry = super::history::GraphHistoryEntry {
            id: format!("h-fork-{}", child.run_id),
            parent_id: Some(parent.run_id.clone()),
            run_id: child.run_id.clone(),
            definition_digest: child.definition_digest.clone(),
            task_checkpoint: preview.checkpoint_ref.clone(),
            attempt_refs: vec![],
            evidence_refs: vec![],
            root_budget_id: Some(parent.run_id.clone()),
            state_revision: child.revision,
            timestamp: store::now_ms(),
        };
        super::history::write_history_entry(&workspace, &entry)
            .map_err(|error| error.to_string())?;
        store::save_run(&mut child).map_err(|error| error.to_string())?;
        Ok(child)
    })();
    if result.is_err() {
        if is_git && workspace.exists() {
            let _ = git::run(
                cwd,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    workspace.to_str().unwrap_or(""),
                ],
            );
        }
        // This container was created exclusively by this call; no parent files.
        let _ = std::fs::remove_dir_all(container);
    }
    result
}
