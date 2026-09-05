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
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONFIG_DIR: &str = ".davinci";
pub const LEGACY_CONFIG_DIR: &str = ".pi";

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
    let davinci = cwd.join(CONFIG_DIR).join("graph").join("runs");
    if davinci.exists() {
        davinci
    } else {
        let pi = cwd.join(LEGACY_CONFIG_DIR).join("graph").join("runs");
        if pi.exists() {
            pi
        } else {
            davinci
        }
    }
}

pub fn run_dir(cwd: &Path, run_id: &str) -> PathBuf {
    runs_root_dir(cwd).join(run_id)
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
fn prune_finished_runs(cwd: &Path) {
    let runs = list_runs(cwd);
    if runs.len() <= RETAINED_RUNS {
        return;
    }
    for run in runs.iter().skip(RETAINED_RUNS) {
        let terminal = matches!(run.phase.as_str(), "done" | "blocked" | "cancelled");
        if terminal && is_safe_run_id(&run.run_id) {
            let _ = fs::remove_dir_all(run_dir(cwd, &run.run_id));
        }
    }
}

/// Publish `content` at `path` without ever leaving a half-written file there.
/// Windows will not replace an existing file through `rename`, so the previous
/// snapshot is moved aside first and restored if the publish fails.
fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".into());
    let temporary = parent.join(format!(".{file_name}.{}.{nonce}.tmp", std::process::id()));
    if let Err(error) = fs::write(&temporary, content) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            let backup = parent.join(format!(".{file_name}.{nonce}.bak"));
            if let Err(error) = fs::rename(path, &backup) {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
            match fs::rename(&temporary, path) {
                Ok(()) => {
                    let _ = fs::remove_file(&backup);
                    Ok(())
                }
                Err(error) => {
                    let _ = fs::rename(&backup, path);
                    let _ = fs::remove_file(&temporary);
                    Err(error)
                }
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

pub fn write_graph_definition(
    cwd: &Path,
    run_id: &str,
    definition: &super::topology::GraphDefinition,
) -> std::io::Result<()> {
    let path = run_dir(cwd, run_id).join("graph.json");
    let content = serde_json::to_vec_pretty(definition)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &content)
}

pub fn load_graph_definition(cwd: &Path, run_id: &str) -> Option<super::topology::GraphDefinition> {
    if !is_safe_run_id(run_id) {
        return None;
    }
    let raw = fs::read_to_string(run_dir(cwd, run_id).join("graph.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_run(run: &mut GraphRun) -> std::io::Result<()> {
    run.updated_at = now_ms();
    let cwd = PathBuf::from(&run.cwd);
    let state_path = run_dir(&cwd, &run.run_id).join("state.json");
    let content = serde_json::to_vec_pretty(run)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    atomic_write(&state_path, &content)?;

    if let Some(definition) = &run.definition {
        let graph_path = run_dir(&cwd, &run.run_id).join("graph.json");
        if !graph_path.exists() {
            let _ = write_graph_definition(&cwd, &run.run_id, definition);
        }
    }
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
    if !is_safe_run_id(run_id) {
        return None;
    }
    let raw = fs::read_to_string(run_dir(cwd, run_id).join("state.json")).ok()?;
    let mut run: GraphRun = serde_json::from_str(&raw).ok()?;
    if run.definition.is_none() {
        run.definition = load_graph_definition(cwd, run_id);
    }
    for task in &mut run.tasks {
        if task.fingerprint.is_none() {
            task.fingerprint = read_task_fingerprint(cwd, run_id, &task.id);
        }
        if task.mutation.is_none() {
            task.mutation = read_task_mutation(cwd, run_id, &task.id);
        }
    }
    (run.version == 1).then_some(run)
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
    let Ok(entries) = fs::read_dir(runs_root_dir(cwd)) else {
        return Vec::new();
    };
    let mut runs: Vec<RunSummary> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
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
pub fn read_transcript(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|content| content.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{
        ArtifactKind, GraphBudgets, GraphCounters, Phase, ReviewDecision, Verdict,
    };
    use tempfile::tempdir;

    fn sample_run(cwd: &Path, run_id: &str, goal: &str) -> GraphRun {
        GraphRun {
            version: 1,
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
            phase: Phase::Classify,
            forced: None,
            dry_run: false,
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
}
