//! Bounded diagnostic history without evicting recovery boundaries.
use super::{history, types::GraphRun};
use std::io;
use std::path::Path;

const PROGRESS_FILES: usize = 16;
const PROGRESS_BYTES: u64 = 16 * 1024 * 1024;
const HISTORY_FILES: usize = 4096;
const HISTORY_BYTES: u64 = 512 * 1024 * 1024;

/// Streaming diagnostics/spend stay in canonical state. Changes to execution,
/// attempts, effects, budgets, input identity or cursor remain recovery boundaries.
pub(super) fn same_boundary(left: &GraphRun, right: &GraphRun) -> bool {
    fn normalize(mut run: GraphRun) -> GraphRun {
        run.updated_at = 0;
        run.checkpoint_sequence = 0;
        run.resource_snapshot = None;
        run.ecosystem_stats = Default::default();
        run.counters.cost_usd = 0.0;
        for task in &mut run.tasks {
            task.usage = Default::default();
            task.last_activity = None;
            task.last_message = None;
        }
        run
    }
    normalize(left.clone()) == normalize(right.clone())
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CheckpointClass {
    #[serde(default)]
    pub progress_only: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Checkpoint<'a> {
    #[serde(flatten)]
    pub run: &'a GraphRun,
    pub progress_only: bool,
}

/// Reserve capacity before publishing any new checkpoint. Only redundant
/// diagnostics may be removed. Exhausted essential capacity applies backpressure.
pub(super) fn reserve(
    cwd: &Path,
    run_id: &str,
    previous: &GraphRun,
    bytes: u64,
    progress: bool,
    archive_revision: bool,
) -> io::Result<()> {
    reserve_with_limits(
        cwd,
        run_id,
        previous,
        bytes,
        progress,
        archive_revision,
        HISTORY_FILES,
        HISTORY_BYTES,
    )
}

#[allow(clippy::too_many_arguments)]
fn reserve_with_limits(
    cwd: &Path,
    run_id: &str,
    previous: &GraphRun,
    bytes: u64,
    progress: bool,
    archive_revision: bool,
    max_files: usize,
    max_bytes: u64,
) -> io::Result<()> {
    let directory = history::history_dir(cwd, run_id);
    let mut files = 0usize;
    let mut total = 0u64;
    let mut diagnostic = Vec::new();
    match std::fs::read_dir(&directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.ends_with(".state.json") {
                    continue;
                }
                let metadata = entry.metadata()?;
                if !entry.file_type()?.is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "non-file checkpoint history entry",
                    ));
                }
                files += 1;
                total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| io::Error::other("history byte count overflow"))?;
                if let Some(sequence) = name
                    .strip_prefix("progress-")
                    .and_then(|name| name.strip_suffix(".state.json"))
                    .and_then(|name| name.parse::<u64>().ok())
                {
                    diagnostic.push((sequence, entry.path(), metadata.len()));
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let prefix = if progress { "progress" } else { "checkpoint" };
    let next = directory.join(format!(
        "{prefix}-{}.state.json",
        previous.checkpoint_sequence
    ));
    let add = usize::from(!next.exists());
    let revision = directory.join(format!("revision-{}.state.json", previous.revision));
    let add_revision = usize::from(archive_revision && !revision.exists());
    let added = add + add_revision;
    let added_bytes = bytes.saturating_mul(added as u64);
    diagnostic.sort_by_key(|(sequence, _, _)| *sequence);
    let mut diagnostic_count = diagnostic.len() + usize::from(progress && add > 0);
    let mut diagnostic_bytes: u64 = diagnostic
        .iter()
        .map(|(_, _, bytes)| *bytes)
        .sum::<u64>()
        .saturating_add(if progress && add > 0 { bytes } else { 0 });
    for (_, path, size) in diagnostic {
        let exceeds = diagnostic_count > PROGRESS_FILES
            || diagnostic_bytes > PROGRESS_BYTES
            || files.saturating_add(added) > max_files
            || total.saturating_add(added_bytes) > max_bytes;
        if !exceeds {
            break;
        }
        std::fs::remove_file(path)?;
        files -= 1;
        total -= size;
        diagnostic_count -= 1;
        diagnostic_bytes -= size;
    }
    if files.saturating_add(added) > max_files || total.saturating_add(added_bytes) > max_bytes {
        return Err(io::Error::other("checkpoint history capacity exhausted; archive the run before continuing (recovery boundaries preserved)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pr_review_history_capacity_preserves_essential_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let run: GraphRun = serde_json::from_value(serde_json::json!({
            "version":1,"dryRun":false,"runId":"quota","goal":"goal","cwd":dir.path(),"phase":"classify",
            "budgets":super::super::types::GraphBudgets::default(),"counters":{"workersSpawned":0,"revisionCycles":0,"replans":0,"costUsd":0.0,"startedAt":0},
            "updatedAt":0
        })).unwrap();
        let history = history::history_dir(dir.path(), &run.run_id);
        std::fs::create_dir_all(&history).unwrap();
        let old = history.join("checkpoint-1.state.json");
        std::fs::write(&old, b"protected").unwrap();
        assert!(
            reserve_with_limits(dir.path(), &run.run_id, &run, 10, false, false, 1, 10).is_err()
        );
        assert_eq!(std::fs::read(old).unwrap(), b"protected");
    }
}
