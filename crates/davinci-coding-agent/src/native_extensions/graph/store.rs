//! Disk layout and persistence for graph runs.
//!
//! ```text
//! <cwd>/.pi/graph/runs/<runId>/
//!    state.json              - GraphRun, atomic write (tmp + rename)
//!    artifacts/<taskId>.json - typed node outputs
//!    logs/<taskId>.log       - worker stderr + final text, diagnostics only
//!    logs/<taskId>.live.log  - append-only transcript, tailed by /graph-view
//! ```

use super::types::{Artifact, ArtifactKind, GraphRun};
use super::validate::validate_artifact;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::history;
#[allow(unused_imports)]
pub use history::*;

pub const CONFIG_DIR: &str = ".davinci";
pub const LEGACY_CONFIG_DIR: &str = ".pi";

fn modern_runs_root_dir(cwd: &Path) -> PathBuf {
    cwd.join(CONFIG_DIR).join("graph").join("runs")
}

fn legacy_runs_root_dir(cwd: &Path) -> PathBuf {
    cwd.join(LEGACY_CONFIG_DIR).join("graph").join("runs")
}

/// Return both graph roots in authority order. The legacy root is a migration
/// source and is never allowed to silently shadow a conflicting modern run.
pub fn graph_run_roots(cwd: &Path) -> Vec<PathBuf> {
    let modern = modern_runs_root_dir(cwd);
    let legacy = legacy_runs_root_dir(cwd);
    if modern == legacy {
        vec![modern]
    } else {
        vec![modern, legacy]
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Epoch milliseconds as an ISO-8601 UTC timestamp, so a transcript read weeks
/// later says when it was written. Uses Howard Hinnant's civil-from-days.
pub fn iso8601_utc(ms: u64) -> String {
    let seconds = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let shifted_month = mp + 3;
    let month = if shifted_month <= 12 {
        shifted_month
    } else {
        shifted_month - 12
    };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

pub fn runs_root_dir(cwd: &Path) -> PathBuf {
    let davinci = modern_runs_root_dir(cwd);
    if davinci.exists() {
        davinci
    } else {
        let pi = legacy_runs_root_dir(cwd);
        if pi.exists() {
            pi
        } else {
            davinci
        }
    }
}

pub fn run_dir(cwd: &Path, run_id: &str) -> PathBuf {
    let modern = modern_runs_root_dir(cwd).join(run_id);
    let legacy = legacy_runs_root_dir(cwd).join(run_id);
    if modern.join("state.json").exists() {
        modern
    } else if legacy.join("state.json").exists() {
        legacy
    } else {
        runs_root_dir(cwd).join(run_id)
    }
}

pub fn is_safe_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

pub fn new_run_id() -> String {
    let millis = now_ms();
    let mut encoded = String::new();
    let mut remaining = millis;
    while remaining > 0 {
        let digit = (remaining % 36) as u32;
        encoded.push(char::from_digit(digit, 36).unwrap_or('0'));
        remaining /= 36;
    }
    if encoded.is_empty() {
        encoded.push('0');
    }
    let timestamp: String = encoded.chars().rev().collect();
    let suffix = uuid::Uuid::new_v4().to_string();
    format!("{timestamp}-{}", &suffix[..8])
}

/// Runs kept when a new one starts; everything is on disk, so the cap only
/// bounds growth, it is not a history feature.
const RETAINED_RUNS: usize = 20;
const MAX_OPERATION_REFERENCE_SCAN_BYTES: u64 = 256 * 1024;

pub fn create_run_dir(cwd: &Path, run_id: &str) -> std::io::Result<()> {
    let root = run_dir(cwd, run_id);
    fs::create_dir_all(root.join("artifacts"))?;
    fs::create_dir_all(root.join("logs"))?;
    prune_finished_runs(cwd);
    Ok(())
}

/// Delete the oldest runs beyond [`RETAINED_RUNS`]. Only runs whose persisted
/// phase is terminal are touched: a live run — including one owned by another
/// process — never is, whatever its age.
#[allow(dead_code)]
pub fn restored_worker_state(recorded: &str, live_identity_verified: bool) -> &str {
    if recorded == "running" && !live_identity_verified {
        "reconciliation_required"
    } else {
        recorded
    }
}

#[allow(dead_code)]
pub fn pin_run(cwd: &Path, run_id: &str) -> std::io::Result<()> {
    let pin_file = run_dir(cwd, run_id).join(".pinned");
    atomic_write(&pin_file, b"1")
}

#[allow(dead_code)]
pub fn is_run_pinned(cwd: &Path, run_id: &str) -> bool {
    run_dir(cwd, run_id).join(".pinned").exists()
}

#[allow(dead_code)]
pub fn record_ancestor_run(cwd: &Path, run_id: &str, ancestor_run_id: &str) -> std::io::Result<()> {
    let file = run_dir(cwd, run_id).join("ancestor_run.txt");
    atomic_write(&file, ancestor_run_id.as_bytes())
}

#[allow(dead_code)]
pub fn read_ancestor_run(cwd: &Path, run_id: &str) -> Option<String> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let file = run_dir(cwd, run_id).join("ancestor_run.txt");
    fs::read_to_string(file).ok().map(|s| s.trim().to_string())
}

fn prune_finished_runs(cwd: &Path) {
    let runs = list_runs(cwd);
    if runs.len() <= RETAINED_RUNS {
        return;
    }
    let mut pinned_ancestors = std::collections::HashSet::new();
    for run in &runs {
        if let Some(ancestor) = read_ancestor_run(cwd, &run.run_id) {
            pinned_ancestors.insert(ancestor);
        }
        let state_path = run_dir(cwd, &run.run_id).join("state.json");
        if let Ok(raw) = fs::read_to_string(&state_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                if let Some(anc) = v.get("ancestorRunId").and_then(|s| s.as_str()) {
                    pinned_ancestors.insert(anc.to_string());
                }
            }
        }
        for entry in list_history_entries(cwd, &run.run_id) {
            if let Some(parent) = entry.parent_id {
                pinned_ancestors.insert(parent);
            }
        }
    }

    for run in runs.iter().skip(RETAINED_RUNS) {
        let terminal = matches!(run.phase.as_str(), "done" | "blocked" | "cancelled");
        let is_pinned = is_run_pinned(cwd, &run.run_id)
            || pinned_ancestors.contains(&run.run_id)
            || run_has_operation_references(cwd, &run.run_id);
        if terminal && !is_pinned && is_safe_run_id(&run.run_id) {
            let _ = fs::remove_dir_all(run_dir(cwd, &run.run_id));
        }
    }
    collect_unreferenced_blobs(cwd);
}

fn collect_unreferenced_blobs(cwd: &Path) {
    let referenced: std::collections::HashSet<String> = list_runs(cwd)
        .iter()
        .filter_map(|summary| load_run(cwd, &summary.run_id))
        .flat_map(|run| run.baseline_hashes())
        .collect();
    let blob_dir = super::blobs::dir(cwd);
    let Ok(shards) = fs::read_dir(&blob_dir) else {
        return;
    };
    for shard in shards.flatten() {
        let Ok(entries) = fs::read_dir(shard.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(shard.path());
    }
}

/// A graph checkpoint can outlive the in-memory projection when an operation
/// is still unresolved or its result has not been acknowledged.  Retention is
/// therefore conservative: any persisted operation binding keeps the run
/// directory until an explicit archive removes the evidence.  An unreadable
/// candidate is retained too, because pruning through an unknown checkpoint
/// would turn uncertainty into data loss.
fn run_has_operation_references(cwd: &Path, run_id: &str) -> bool {
    let root = run_dir(cwd, run_id);
    let mut candidates = vec![root.join("state.json")];
    let artifacts = root.join("artifacts");
    let entries = match fs::read_dir(&artifacts) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            candidates.push(path);
        }
    }

    candidates.into_iter().any(|path| {
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => return true,
        };
        let mut bytes = Vec::new();
        let mut bounded = (&mut file).take(MAX_OPERATION_REFERENCE_SCAN_BYTES + 1);
        if bounded.read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 > MAX_OPERATION_REFERENCE_SCAN_BYTES
        {
            return true;
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => json_contains_operation_reference(&value),
            Err(_) => true,
        }
    })
}

fn json_contains_operation_reference(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(json_contains_operation_reference),
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "operationBinding"
                    | "operation_binding"
                    | "operationReference"
                    | "operation_reference"
                    | "launchOperationId"
                    | "launch_operation_id"
                    | "launchAttemptId"
                    | "launch_attempt_id"
            ) && !value.is_null()
                || json_contains_operation_reference(value)
        }),
        _ => false,
    }
}

/// Publish `content` at `path` without ever leaving a half-written file there.
/// Flush the new image before replacing the previous one in a single rename.
/// A failed replacement must leave the previous checkpoint at its original name.
pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    davinci_sys::fs::atomic_write(path, content)
}

pub fn load_graph_definition(cwd: &Path, run_id: &str) -> Option<super::topology::GraphDefinition> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let raw = fs::read_to_string(run_dir(cwd, run_id).join("graph.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn migrate_baseline(
    baseline: &mut super::mutation::MutationBaseline,
    blob_dir: &Path,
) -> std::io::Result<()> {
    for (path, bytes) in std::mem::take(&mut baseline.contents) {
        if let Some(fingerprint) = baseline.files.get(&path) {
            super::blobs::put(blob_dir, &fingerprint.hash, &bytes)?;
        }
    }
    Ok(())
}

fn migrate_inline_baselines(run: &mut GraphRun, blob_dir: &Path) -> std::io::Result<()> {
    let Some(cursor) = run.continuation.as_mut() else {
        return Ok(());
    };
    if let Some(delivery) = cursor.delivery.as_mut() {
        migrate_baseline(&mut delivery.baseline, blob_dir)?;
        if let Some(baseline) = delivery.attempt_baseline.as_mut() {
            migrate_baseline(baseline, blob_dir)?;
        }
    }
    if let Some(delivery) = cursor.completed_delivery.as_mut() {
        migrate_baseline(&mut delivery.baseline, blob_dir)?;
        if let Some(baseline) = delivery.attempt_baseline.as_mut() {
            migrate_baseline(baseline, blob_dir)?;
        }
    }
    if let Some(baseline) = cursor.saved_baseline.as_mut() {
        migrate_baseline(baseline, blob_dir)?;
    }
    for baseline in cursor.saved_attempt_baselines.values_mut() {
        migrate_baseline(baseline, blob_dir)?;
    }
    Ok(())
}

pub fn save_run(run: &mut GraphRun) -> std::io::Result<()> {
    let cwd = PathBuf::from(&run.cwd);
    let state_path = run_dir(&cwd, &run.run_id).join("state.json");

    if let Some(definition) = &run.definition {
        let graph_path = run_dir(&cwd, &run.run_id).join("graph.json");
        let bytes = serde_json::to_vec_pretty(definition)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if fs::read(&graph_path).ok().as_deref() != Some(bytes.as_slice()) {
            atomic_write(&graph_path, &bytes)?;
        }
    }

    if let Some(saved_def) = &run.saved_definition {
        let saved_path = run_dir(&cwd, &run.run_id).join("saved_definition.yaml");
        if !saved_path.exists() {
            let yaml_str = super::definitions::to_yaml_string(saved_def);
            atomic_write(&saved_path, yaml_str.as_bytes())?;
        }
    }
    // Migrate legacy inline bytes before cloning so a resumed old run does
    // not keep a second repository-sized copy in memory. Blob writes are
    // content-addressed and atomic, so clearing the inline copy is safe once
    // this step succeeds even if publishing state.json later fails.
    let blob_dir = super::blobs::dir(&cwd);
    migrate_inline_baselines(run, &blob_dir)?;
    // The self-contained run state is the commit record. Publish it only after
    // all companion writes succeed, and retain the previous timestamp on error.
    let mut snapshot = run.clone();
    snapshot.updated_at = now_ms();
    let content = serde_json::to_vec(&snapshot)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if let Ok(previous_bytes) = fs::read(&state_path) {
        if let Ok(previous) = serde_json::from_slice::<GraphRun>(&previous_bytes) {
            if previous.run_id == snapshot.run_id && previous.revision < snapshot.revision {
                let history_path = super::history::history_dir(&cwd, &run.run_id)
                    .join(format!("revision-{}.state.json", previous.revision));
                if !history_path.exists() {
                    atomic_write(&history_path, &previous_bytes)?;
                }
            }
        }
    }
    atomic_write(&state_path, &content)?;
    run.updated_at = snapshot.updated_at;
    Ok(())
}

pub fn write_task_fingerprint(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    fingerprint: &super::replay::ReplayFingerprint,
) -> std::io::Result<()> {
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.fingerprint.json"));
    let content = serde_json::to_vec_pretty(fingerprint)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &content)
}

pub fn read_task_fingerprint(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
) -> Option<super::replay::ReplayFingerprint> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.fingerprint.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAttemptRecord {
    pub task_id: String,
    pub attempt: u32,
    pub status: super::types::TaskStatus,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_pid: Option<u32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub run_deadline_exceeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub usage: super::types::WorkerUsage,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<super::replay::ReplayFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_session: Option<super::worker_sessions::WorkerSessionBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_binding: Option<super::operation_bridge::GraphOperationBinding>,
    /// Durable explanation for the automatic retry gate.  This is attached
    /// to the failed attempt so a restart can replay the same decision without
    /// reclassifying diagnostic text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_recovery: Option<super::recovery::RetryRecoveryRecord>,
}

pub fn write_task_attempt(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    attempt: u32,
    record: &TaskAttemptRecord,
) -> std::io::Result<()> {
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.attempt_{attempt}.json"));
    let content = serde_json::to_vec_pretty(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &content)
}

#[allow(dead_code)]
pub fn read_task_attempt(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    attempt: u32,
) -> Option<TaskAttemptRecord> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.attempt_{attempt}.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_task_mutation(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    mutation: &super::mutation::GraphMutation,
) -> std::io::Result<()> {
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.mutation.json"));
    let content = serde_json::to_vec_pretty(mutation)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &content)
}

pub fn read_task_mutation(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
) -> Option<super::mutation::GraphMutation> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.mutation.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[allow(dead_code)]
pub fn write_task_context_packet(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    packet: &crate::native_extensions::ecosystem::ContextPacket,
) -> std::io::Result<()> {
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.context.json"));
    let content = serde_json::to_vec_pretty(packet)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &content)
}

#[allow(dead_code)]
pub fn read_task_context_packet(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
) -> Option<crate::native_extensions::ecosystem::ContextPacket> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let path = run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.context.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn load_run(cwd: &Path, run_id: &str) -> Option<GraphRun> {
    load_run_checked(cwd, run_id).ok()
}

pub fn load_run_checked(cwd: &Path, run_id: &str) -> Result<GraphRun, String> {
    if !is_safe_run_id(run_id) {
        return Err("Invalid graph run identity; expected a path-safe run ID.".into());
    }
    let modern_state = modern_runs_root_dir(cwd).join(run_id).join("state.json");
    let legacy_state = legacy_runs_root_dir(cwd).join(run_id).join("state.json");
    if modern_state.is_file() && legacy_state.is_file() {
        let modern = fs::read(&modern_state).map_err(|error| {
            format!("Cannot read modern checkpoint for run '{run_id}': {error}")
        })?;
        let legacy = fs::read(&legacy_state).map_err(|error| {
            format!("Cannot read legacy checkpoint for run '{run_id}': {error}")
        })?;
        if modern != legacy {
            return Err(format!(
                "conflicting .davinci and legacy .pi checkpoints exist for run '{run_id}'; explicit migration is required"
            ));
        }
    }
    let raw = fs::read_to_string(run_dir(cwd, run_id).join("state.json"))
        .map_err(|error| format!("Cannot read checkpoint for run '{run_id}': {error}"))?;
    let mut run: GraphRun = serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid checkpoint for run '{run_id}': {error}"))?;
    if run.version != 1 {
        return Err(format!(
            "Unsupported graph checkpoint version {} for run '{run_id}'.",
            run.version
        ));
    }
    if run.run_id != run_id {
        return Err(format!(
            "Checkpoint run identity does not match requested run '{run_id}'."
        ));
    }
    let expected_cwd = cwd
        .canonicalize()
        .map_err(|error| format!("Cannot resolve requested graph workspace: {error}"))?;
    let stored_cwd = Path::new(&run.cwd).canonicalize().map_err(|error| {
        format!("Cannot resolve checkpoint workspace for run '{run_id}': {error}")
    })?;
    if expected_cwd != stored_cwd {
        return Err(format!("Checkpoint for run '{run_id}' belongs to a different workspace. Open it from its original workspace."));
    }
    if run.definition.is_none() {
        run.definition = load_graph_definition(cwd, run_id);
    }
    if run.saved_definition.is_none() {
        let saved_path = run_dir(cwd, run_id).join("saved_definition.yaml");
        if saved_path.exists() {
            let raw_yaml = fs::read_to_string(&saved_path).map_err(|error| {
                format!(
                    "Cannot read saved graph definition '{}': {error}",
                    saved_path.display()
                )
            })?;
            let def = super::definitions::parse_saved_definition(&raw_yaml).map_err(|error| {
                format!(
                    "Cannot parse saved graph definition '{}': {error}",
                    saved_path.display()
                )
            })?;
            run.saved_definition = Some(def);
        }
    }
    for task in &mut run.tasks {
        if task.fingerprint.is_none() {
            task.fingerprint = read_task_fingerprint(cwd, run_id, &task.id);
        }
        if task.mutation.is_none() {
            task.mutation = read_task_mutation(cwd, run_id, &task.id);
        }
    }
    Ok(run)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub phase: String,
    pub goal: String,
    pub updated_at: u64,
    pub cost_usd: f64,
    pub workers_spawned: u32,
}

/// Every persisted run in this project, newest first.
pub fn list_runs(cwd: &Path) -> Vec<RunSummary> {
    let mut run_ids = std::collections::BTreeSet::new();
    for root in graph_run_roots(cwd) {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        run_ids.extend(
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string)),
        );
    }
    let mut runs: Vec<RunSummary> = run_ids
        .into_iter()
        .filter_map(|run_id| load_run(cwd, &run_id))
        .map(|run| RunSummary {
            run_id: run.run_id,
            phase: run.phase.as_str().to_string(),
            goal: run.goal,
            updated_at: run.updated_at,
            cost_usd: run.counters.cost_usd,
            workers_spawned: run.counters.workers_spawned,
        })
        .collect();
    runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    runs
}

pub fn artifact_path(cwd: &Path, run_id: &str, task_id: &str) -> PathBuf {
    run_dir(cwd, run_id)
        .join("artifacts")
        .join(format!("{task_id}.json"))
}

/// Live, append-only transcript a worker writes as it runs; `/graph-view` tails it.
pub fn transcript_path(cwd: &Path, run_id: &str, task_id: &str) -> PathBuf {
    run_dir(cwd, run_id)
        .join("logs")
        .join(format!("{task_id}.live.log"))
}

pub fn write_artifact(path: &Path, artifact: &Artifact) -> std::io::Result<()> {
    let content = serde_json::to_vec_pretty(artifact)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(path, &content)
}

pub fn read_artifact(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    expect: ArtifactKind,
) -> Result<Artifact, Vec<String>> {
    let path = artifact_path(cwd, run_id, task_id);
    let raw = fs::read_to_string(&path).map_err(|_| {
        vec![format!(
            "artifact file for task \"{task_id}\" does not exist"
        )]
    })?;
    let parsed: Value = serde_json::from_str(&raw).map_err(|_| {
        vec![format!(
            "artifact file for task \"{task_id}\" is not valid JSON"
        )]
    })?;
    validate_artifact(expect, &parsed)
}

/// Diagnostics only — never let logging kill a run.
pub fn write_log(cwd: &Path, run_id: &str, task_id: &str, content: &str) {
    let path = run_dir(cwd, run_id)
        .join("logs")
        .join(format!("{task_id}.log"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, content);
}

/// The tail of a worker's live transcript, for `/graph-view`.
/// The live inspector must not reread an entire growing log every second.
pub fn read_transcript_tail(path: &Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let read = || -> std::io::Result<Vec<String>> {
        let mut file = fs::File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(64 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(64 * 1024).read_to_end(&mut bytes)?;
        // A tail can start inside a UTF-8 code point or a partial log line.
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<_> = text.lines().skip(usize::from(start > 0)).collect();
        Ok(lines
            .into_iter()
            .rev()
            .take(200)
            .rev()
            .map(str::to_owned)
            .collect())
    };
    read().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{
        ArtifactKind, GraphBudgets, GraphCounters, Phase, ReviewDecision, Verdict,
    };
    use tempfile::tempdir;

    #[test]
    fn blob_gc_keeps_referenced_hashes_and_removes_orphans() {
        use crate::native_extensions::graph::continuation::GraphContinuation;
        use crate::native_extensions::graph::mutation::{FileFingerprint, MutationBaseline};
        use std::collections::BTreeMap;

        let dir = tempdir().unwrap();
        let run_id = "blob-gc";
        create_run_dir(dir.path(), run_id).unwrap();
        let keep_bytes = b"keep me";
        let orphan_bytes = b"remove me";
        let keep_hash = crate::native_extensions::graph::replay::sha256_hex(keep_bytes);
        let orphan_hash = crate::native_extensions::graph::replay::sha256_hex(orphan_bytes);
        let blob_dir = super::super::blobs::dir(dir.path());
        super::super::blobs::put(&blob_dir, &keep_hash, keep_bytes).unwrap();
        super::super::blobs::put(&blob_dir, &orphan_hash, orphan_bytes).unwrap();

        let mut run = sample_run(dir.path(), run_id, "goal");
        let mut files = BTreeMap::new();
        files.insert(
            "a.txt".into(),
            FileFingerprint {
                hash: keep_hash.clone(),
                len: keep_bytes.len() as u64,
            },
        );
        run.continuation = Some(GraphContinuation {
            saved_baseline: Some(MutationBaseline {
                files,
                contents: BTreeMap::new(),
            }),
            ..GraphContinuation::default()
        });
        save_run(&mut run).unwrap();

        collect_unreferenced_blobs(dir.path());
        assert_eq!(
            super::super::blobs::get(&blob_dir, &keep_hash).unwrap(),
            keep_bytes
        );
        assert!(super::super::blobs::get(&blob_dir, &orphan_hash).is_none());
    }

    #[test]
    fn live_transcript_tail_is_bounded_and_preserves_recent_unicode() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("worker.live.log");
        assert!(read_transcript_tail(&path).is_empty());
        fs::write(
            &path,
            format!(
                "{}\nrecent 界 activity\n",
                "old 界 activity\n".repeat(10000)
            ),
        )
        .unwrap();
        let tail = read_transcript_tail(&path);
        assert_eq!(tail.len(), 200);
        assert_eq!(tail.last().unwrap(), "recent 界 activity");
        assert!(tail.iter().all(|line| !line.contains('\u{fffd}')));
    }

    fn sample_run(cwd: &Path, run_id: &str, goal: &str) -> GraphRun {
        GraphRun {
            version: 1,
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
            phase: Phase::Classify,
            forced: None,
            dry_run: false,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: Vec::new(),
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: now_ms(),
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: None,
            revision: 0,
            control_history: Vec::new(),
            continuation: None,
        }
    }

    #[test]
    fn a_saved_run_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();
        let mut run = sample_run(dir.path(), &run_id, "goal text");
        save_run(&mut run).unwrap();
        let loaded = load_run(dir.path(), &run_id).expect("loads");
        assert_eq!(loaded.goal, "goal text");
        assert_eq!(loaded.budgets, GraphBudgets::default());
        assert!(loaded.updated_at > 0);
    }

    #[test]
    fn saved_state_is_compact_and_has_no_inline_contents() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();
        let mut run = sample_run(dir.path(), &run_id, "goal");
        save_run(&mut run).unwrap();
        let raw = fs::read_to_string(run_dir(dir.path(), &run_id).join("state.json")).unwrap();
        assert!(!raw.contains("\n  "), "state.json must be compact");
        assert!(!raw.contains("\"contents\""));
    }

    #[test]
    fn saving_twice_replaces_the_previous_snapshot() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();
        let mut run = sample_run(dir.path(), &run_id, "first");
        save_run(&mut run).unwrap();
        run.goal = "second".into();
        save_run(&mut run).unwrap();
        assert_eq!(load_run(dir.path(), &run_id).unwrap().goal, "second");
    }

    #[test]
    fn failed_companion_write_does_not_publish_new_run_state() {
        let dir = tempdir().unwrap();
        let mut run = sample_run(dir.path(), "fixture", "durable goal");
        save_run(&mut run).unwrap();
        let state = run_dir(dir.path(), &run.run_id).join("state.json");
        let before = fs::read(&state).unwrap();
        let timestamp = run.updated_at;
        let graph = run_dir(dir.path(), &run.run_id).join("graph.json");
        fs::create_dir(&graph).unwrap();
        fs::write(graph.join("block"), "fixture").unwrap();
        run.goal = "uncommitted goal".into();
        run.definition = Some(super::super::topology::GraphDefinition {
            graph_id: "fixture".into(),
            version: 1,
            mode: super::super::topology::GraphMode::Simple,
            nodes: vec![],
            edges: vec![],
        });
        assert!(save_run(&mut run).is_err());
        assert_eq!(fs::read(state).unwrap(), before);
        assert_eq!(run.updated_at, timestamp);
    }

    #[test]
    fn failed_replace_never_moves_the_previous_checkpoint_aside() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"old").unwrap();
        let mut calls = 0;
        let result = atomic_write_with(&path, b"new", |_, _| {
            calls += 1;
            assert_eq!(
                calls, 1,
                "a failed atomic replace must not fall back to multiple renames"
            );
            assert_eq!(fs::read(&path).unwrap(), b"old");
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture",
            ))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_write_restores_old_content_when_publish_is_interrupted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"old").unwrap();
        let mut calls = 0;
        let result = atomic_write_with(&path, b"new", |from, to| {
            calls += 1;
            if calls == 1 || calls == 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "simulated interrupted publish",
                ))
            } else {
                fs::rename(from, to)
            }
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert!(fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".bak")));
    }

    #[test]
    fn runs_are_listed_newest_first() {
        let dir = tempdir().unwrap();
        for (index, goal) in ["older", "newer"].into_iter().enumerate() {
            let run_id = format!("run-{index}");
            create_run_dir(dir.path(), &run_id).unwrap();
            let mut run = sample_run(dir.path(), &run_id, goal);
            run.updated_at = index as u64;
            save_run(&mut run).unwrap();
            // save_run stamps updated_at itself; force a distinct order.
            let path = run_dir(dir.path(), &run_id).join("state.json");
            let mut stored: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            stored["updatedAt"] = serde_json::json!(index as u64 + 1);
            fs::write(&path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
        }
        let runs = list_runs(dir.path());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].goal, "newer");
    }

    #[test]
    fn artifacts_are_validated_on_the_way_back_in() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();
        let artifact = Artifact::Review(Box::new(ReviewDecision {
            verdict: Verdict::Approve,
            issues: Vec::new(),
            notes: "fine".into(),
            reviewed_chunk_ids: Vec::new(),
        }));
        write_artifact(&artifact_path(dir.path(), &run_id, "review-1"), &artifact).unwrap();
        let loaded =
            read_artifact(dir.path(), &run_id, "review-1", ArtifactKind::Review).expect("valid");
        assert_eq!(loaded.as_review().unwrap().verdict, Verdict::Approve);

        fs::write(artifact_path(dir.path(), &run_id, "bad"), "{}").unwrap();
        assert!(read_artifact(dir.path(), &run_id, "bad", ArtifactKind::Review).is_err());
    }

    #[test]
    fn epoch_millis_render_as_iso_8601_utc() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_000), "1970-01-01T00:00:01.000Z");
        assert_eq!(iso8601_utc(1_788_104_079_828), "2026-08-30T15:34:39.828Z");
        // A leap day, to exercise the civil-from-days branch.
        assert_eq!(iso8601_utc(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn run_ids_are_path_safe_and_unique() {
        let first = new_run_id();
        let second = new_run_id();
        assert_ne!(first, second);
        assert!(is_safe_run_id(&first));
        assert!(!is_safe_run_id("../escape"));
        assert!(!is_safe_run_id(""));
    }

    #[test]
    fn an_unsafe_run_id_never_reaches_the_filesystem() {
        let dir = tempdir().unwrap();
        assert!(load_run(dir.path(), "../../etc/passwd").is_none());
    }

    #[test]
    fn loaded_run_is_bound_to_requested_workspace_and_identity() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let mut run = sample_run(dir.path(), "bound-run", "fixture");
        save_run(&mut run).unwrap();
        let path = run_dir(dir.path(), "bound-run").join("state.json");
        let original = fs::read(&path).unwrap();
        for (field, value) in [
            ("cwd", other.path().to_string_lossy().into_owned()),
            ("runId", "other-run".into()),
        ] {
            let mut state: Value = serde_json::from_slice(&original).unwrap();
            state[field] = Value::String(value);
            fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
            assert!(load_run(dir.path(), "bound-run").is_none(), "{field}");
        }
        fs::write(&path, original).unwrap();
        assert!(load_run(&dir.path().join("."), "bound-run").is_some());
    }

    #[test]
    fn checked_load_explains_missing_corrupt_and_incompatible_checkpoints() {
        let dir = tempdir().unwrap();
        assert!(load_run_checked(dir.path(), "missing")
            .unwrap_err()
            .contains("Cannot read checkpoint"));
        let mut run = sample_run(dir.path(), "bad-run", "fixture");
        save_run(&mut run).unwrap();
        let path = run_dir(dir.path(), "bad-run").join("state.json");
        fs::write(&path, b"{").unwrap();
        assert!(load_run_checked(dir.path(), "bad-run")
            .unwrap_err()
            .contains("Invalid checkpoint"));
        run.version = 2;
        fs::write(&path, serde_json::to_vec(&run).unwrap()).unwrap();
        assert!(load_run_checked(dir.path(), "bad-run")
            .unwrap_err()
            .contains("Unsupported graph checkpoint version 2"));
    }

    #[test]
    fn legacy_graph_root_is_visible_and_conflicts_fail_closed() {
        let dir = tempdir().unwrap();
        let legacy_run_id = "legacy-run";
        fs::create_dir_all(legacy_runs_root_dir(dir.path())).unwrap();
        let mut legacy = sample_run(dir.path(), legacy_run_id, "legacy");
        save_run(&mut legacy).unwrap();
        assert!(run_dir(dir.path(), legacy_run_id).starts_with(dir.path().join(LEGACY_CONFIG_DIR)));
        assert_eq!(list_runs(dir.path()).len(), 1);

        let modern = modern_runs_root_dir(dir.path()).join(legacy_run_id);
        fs::create_dir_all(&modern).unwrap();
        fs::write(
            modern.join("state.json"),
            br#"{"version":1,"runId":"legacy-run"}"#,
        )
        .unwrap();
        let error = load_run_checked(dir.path(), legacy_run_id).unwrap_err();
        assert!(error.contains("conflicting .davinci and legacy .pi checkpoints"));
    }

    #[test]
    fn graph_definition_roundtrips_through_disk_and_sibling_file() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let classification = crate::native_extensions::graph::types::Classification {
            task_class: crate::native_extensions::graph::types::TaskClass::Feature,
            complexity: crate::native_extensions::graph::types::Complexity::Standard,
            rationale: "test".into(),
            research_tasks: vec![crate::native_extensions::graph::types::ResearchRequest {
                kind: crate::native_extensions::graph::types::ResearchKind::CodeSearch,
                focus: "search".into(),
            }],
            milestones: None,
        };

        let def = crate::native_extensions::graph::topology::build_definition(
            crate::native_extensions::graph::topology::GraphMode::Standard,
            &classification,
        );

        let mut run = sample_run(dir.path(), &run_id, "test def roundtrip");
        run.definition = Some(def.clone());
        save_run(&mut run).unwrap();

        // 1. Verify graph.json sibling file was created
        let sibling_def = load_graph_definition(dir.path(), &run_id).expect("graph.json exists");
        assert_eq!(sibling_def, def);

        // 2. Verify state.json loaded run carries the definition
        let reloaded = load_run(dir.path(), &run_id).expect("run loaded");
        assert_eq!(reloaded.definition.as_ref(), Some(&def));
    }

    #[test]
    fn task_attempt_roundtrips_through_disk() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let record = TaskAttemptRecord {
            task_id: "research-1".into(),
            attempt: 1,
            status: crate::native_extensions::graph::types::TaskStatus::Failed,
            exit_code: Some(1),
            child_pid: None,
            timed_out: false,
            run_deadline_exceeded: false,
            artifact_file: None,
            error: Some("test error".into()),
            usage: crate::native_extensions::graph::types::WorkerUsage::default(),
            started_at: Some(100),
            ended_at: Some(200),
            fingerprint: None,
            worker_session: None,
            operation_binding: None,
            retry_recovery: None,
        };

        write_task_attempt(dir.path(), &run_id, "research-1", 1, &record).unwrap();
        let reloaded =
            read_task_attempt(dir.path(), &run_id, "research-1", 1).expect("attempt found");
        assert_eq!(reloaded, record);
    }

    #[test]
    fn f13_restart_never_resurrects_worker() {
        assert_eq!(
            restored_worker_state("running", false),
            "reconciliation_required"
        );
        assert_eq!(restored_worker_state("succeeded", false), "succeeded");
        assert_eq!(restored_worker_state("running", true), "running");
    }

    #[test]
    fn f13_crash_before_after_pause_commit() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = sample_run(dir.path(), &run_id, "pause crash test");
        run.lifecycle = Some(crate::native_extensions::graph::types::GraphLifecycle::Running);
        save_run(&mut run).unwrap();

        // 1. Crash before pause commit: in-memory is pause requested, but on disk it's still running
        let reloaded = load_run(dir.path(), &run_id).unwrap();
        assert_eq!(
            reloaded.current_lifecycle(),
            crate::native_extensions::graph::types::GraphLifecycle::Running
        );

        // 2. Crash after pause commit: disk state was committed with Paused
        run.lifecycle = Some(crate::native_extensions::graph::types::GraphLifecycle::Paused);
        save_run(&mut run).unwrap();

        let reloaded_after = load_run(dir.path(), &run_id).unwrap();
        assert_eq!(
            reloaded_after.current_lifecycle(),
            crate::native_extensions::graph::types::GraphLifecycle::Paused
        );
    }

    #[test]
    fn f13_stop_receipt_after_restart() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = sample_run(dir.path(), &run_id, "stop test");
        run.lifecycle = Some(crate::native_extensions::graph::types::GraphLifecycle::Stopped);
        save_run(&mut run).unwrap();

        let mut reloaded = load_run(dir.path(), &run_id).unwrap();
        let mut tracker = crate::native_extensions::graph::control::ControlTracker::new();
        let control = crate::native_extensions::graph::control::GraphControl {
            operation_id: "op-stop-restart".into(),
            run_id: run_id.clone(),
            expected_run_revision: 0,
            node_id: None,
            expected_attempt: None,
            action: crate::native_extensions::graph::control::GraphControlAction::StopGraph,
        };
        let receipt = crate::native_extensions::graph::control::reduce_control(
            &mut reloaded,
            &control,
            &mut tracker,
            0,
            true,
        );
        assert_eq!(
            reloaded.current_lifecycle(),
            crate::native_extensions::graph::types::GraphLifecycle::Stopped
        );
        assert_eq!(receipt.operation_id, "op-stop-restart");
    }

    #[test]
    fn f13_corrupt_state() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let state_path = run_dir(dir.path(), &run_id).join("state.json");
        fs::write(&state_path, "{ broken json ... ").unwrap();

        assert!(load_run(dir.path(), &run_id).is_none());
        let runs = list_runs(dir.path());
        assert!(runs.is_empty());
    }

    #[test]
    fn f13_terminal_retention_with_referenced_ancestor() {
        let dir = tempdir().unwrap();
        let ancestor_id = "run-ancestor-000".to_string();
        create_run_dir(dir.path(), &ancestor_id).unwrap();
        let mut ancestor_run = sample_run(dir.path(), &ancestor_id, "ancestor run");
        ancestor_run.phase = Phase::Done;
        ancestor_run.lifecycle =
            Some(crate::native_extensions::graph::types::GraphLifecycle::Stopped);
        ancestor_run.updated_at = 1;
        save_run(&mut ancestor_run).unwrap();

        for i in 1..=25 {
            let id = format!("run-child-{:03}", i);
            create_run_dir(dir.path(), &id).unwrap();
            let mut run = sample_run(dir.path(), &id, &format!("run {i}"));
            run.phase = Phase::Done;
            run.lifecycle = Some(crate::native_extensions::graph::types::GraphLifecycle::Stopped);
            run.updated_at = 100 + i as u64;
            save_run(&mut run).unwrap();
            record_ancestor_run(dir.path(), &id, &ancestor_id).unwrap();
        }

        let latest_id = "run-latest".to_string();
        create_run_dir(dir.path(), &latest_id).unwrap();

        assert!(
            run_dir(dir.path(), &ancestor_id).exists(),
            "ancestor run must be retained"
        );
        assert!(load_run(dir.path(), &ancestor_id).is_some());
        assert!(
            !run_dir(dir.path(), "run-child-001").exists(),
            "unreferenced old run should be pruned"
        );
    }

    #[test]
    fn operation_referenced_terminal_run_is_retained_until_archived() {
        let dir = tempdir().unwrap();
        let operation_run_id = "run-operation-000";
        create_run_dir(dir.path(), operation_run_id).unwrap();
        let mut operation_run = sample_run(dir.path(), operation_run_id, "operation evidence");
        operation_run.phase = Phase::Done;
        operation_run.lifecycle =
            Some(crate::native_extensions::graph::types::GraphLifecycle::Stopped);
        save_run(&mut operation_run).unwrap();
        let operation_attempt = run_dir(dir.path(), operation_run_id)
            .join("artifacts")
            .join("research-1.attempt_1.json");
        fs::write(
            operation_attempt,
            br#"{"operationBinding":{"launchOperationId":"op-retain"}}"#,
        )
        .unwrap();

        for i in 1..=25 {
            let id = format!("run-old-{i:03}");
            create_run_dir(dir.path(), &id).unwrap();
            let mut run = sample_run(dir.path(), &id, &format!("old run {i}"));
            run.phase = Phase::Done;
            run.lifecycle = Some(crate::native_extensions::graph::types::GraphLifecycle::Stopped);
            save_run(&mut run).unwrap();
        }

        create_run_dir(dir.path(), "run-latest").unwrap();

        assert!(
            run_dir(dir.path(), operation_run_id).exists(),
            "operation-bound graph evidence must not be pruned"
        );
        assert!(
            !run_dir(dir.path(), "run-old-001").exists(),
            "unreferenced terminal runs remain eligible for pruning"
        );
    }

    #[test]
    fn f13_nonzero_usage_retained_exactly() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = sample_run(dir.path(), &run_id, "usage retention");
        let mut task = crate::native_extensions::graph::types::GraphTaskState::new(
            "research-1",
            crate::native_extensions::graph::types::Role::Researcher,
            ArtifactKind::Evidence,
            vec![],
            None,
        );
        task.usage = crate::native_extensions::graph::types::WorkerUsage {
            input: 12345,
            output: 678,
            cache_read: 50,
            cache_write: 100,
            cost_usd: 0.045,
            turns: 1,
        };
        task.status = crate::native_extensions::graph::types::TaskStatus::Failed;
        run.tasks.push(task);
        run.counters.cost_usd = 0.045;
        save_run(&mut run).unwrap();

        let mut tracker = crate::native_extensions::graph::control::ControlTracker::new();
        let control = crate::native_extensions::graph::control::GraphControl {
            operation_id: "op-retry-usage".into(),
            run_id: run_id.clone(),
            expected_run_revision: 0,
            node_id: Some("research-1".into()),
            expected_attempt: Some(0),
            action: crate::native_extensions::graph::control::GraphControlAction::RetryNode,
        };
        let receipt = crate::native_extensions::graph::control::reduce_control(
            &mut run,
            &control,
            &mut tracker,
            0,
            true,
        );
        assert_eq!(
            receipt.state,
            crate::native_extensions::graph::control::ControlReceiptState::Applied
        );

        assert_eq!(run.total_input(), 12345);
        assert_eq!(run.total_output(), 678);
        assert_eq!(run.counters.cost_usd, 0.045);
    }

    #[test]
    fn f13_legacy_stopped_run_continuation_regression() {
        let dir = tempdir().unwrap();
        let run_id = new_run_id();
        create_run_dir(dir.path(), &run_id).unwrap();

        let mut run = sample_run(dir.path(), &run_id, "legacy done run");
        run.phase = Phase::Done;
        run.lifecycle = None;
        save_run(&mut run).unwrap();

        let loaded = load_run(dir.path(), &run_id).unwrap();
        assert_eq!(
            loaded.current_lifecycle(),
            crate::native_extensions::graph::types::GraphLifecycle::Stopped
        );
        assert!(loaded.current_lifecycle().is_terminal());
    }
}
