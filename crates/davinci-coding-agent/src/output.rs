use std::io::{self, ErrorKind, Write};
use std::thread;
use std::time::Duration;

const RAW_STDOUT_RETRY_DELAY_MS: u64 = 10;

pub fn is_stdout_backpressure(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::WouldBlock | ErrorKind::Interrupted | ErrorKind::WriteZero
    ) || matches!(err.raw_os_error(), Some(11 | 35 | 55))
}

pub fn write_raw_stdout(text: &str) -> io::Result<()> {
    if matches!(
        std::env::var("PI_STDOUT_BACKPRESSURE").as_deref(),
        Ok("1") | Ok("true")
    ) {
        eprintln!("stdout-backpressure-retry");
    }
    let mut out = io::stdout();
    loop {
        match out.write_all(text.as_bytes()).and_then(|_| out.flush()) {
            Ok(()) => return Ok(()),
            Err(err) if is_stdout_backpressure(&err) => {
                thread::sleep(Duration::from_millis(RAW_STDOUT_RETRY_DELAY_MS));
            }
            Err(err) => return Err(err),
        }
    }
}

pub fn write_raw_stdout_line(text: &str) -> io::Result<()> {
    write_raw_stdout(&format!("{text}\n"))
}

/// Writes `contents` to `path` through a temporary file in the same directory
/// and a rename, so a reader never sees a half-written file and a failed write
/// leaves any previous file intact. `--output-last-message` uses it.
pub fn write_file_atomically(path: &std::path::Path, contents: &str) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} does not name a file", path.display()),
        )
    })?;
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => std::path::Path::new("."),
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.{nanos}.tmp", std::process::id()));
    let temp = dir.join(temp_name);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputDimensionSummary {
    pub dimension: String,
    pub state: String,
    pub evidence_label: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputCompletionSummary {
    pub task_id: String,
    pub allowed: bool,
    pub source_fingerprint: String,
    pub evaluated_at_ms: i64,
    pub dimensions: Vec<OutputDimensionSummary>,
    pub remaining_gaps: Vec<String>,
}

#[allow(dead_code)]
impl OutputCompletionSummary {
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Completion Status: {}\n",
            if self.allowed {
                "VERIFIED"
            } else {
                "INCOMPLETE"
            }
        ));
        out.push_str(&format!(
            "Source Fingerprint: {}\n",
            self.source_fingerprint
        ));
        out.push_str("Dimensions:\n");
        for dim in &self.dimensions {
            out.push_str(&format!(
                "  - {}: {} ({})\n",
                dim.dimension,
                dim.evidence_label,
                if dim.required { "required" } else { "optional" }
            ));
            if let Some(ref p) = dim.proof {
                out.push_str(&format!("    Proof: {p}\n"));
            }
        }
        if !self.remaining_gaps.is_empty() {
            out.push_str("Remaining Gaps:\n");
            for gap in &self.remaining_gaps {
                out.push_str(&format!("  ! {gap}\n"));
            }
        }
        out
    }
}

#[allow(dead_code)]
pub fn print_completion_json(summary: &OutputCompletionSummary) -> io::Result<()> {
    let json = summary
        .to_json_string()
        .map_err(|e| io::Error::other(e.to_string()))?;
    write_raw_stdout_line(&json)
}

#[allow(dead_code)]
pub fn print_completion_plain(summary: &OutputCompletionSummary) -> io::Result<()> {
    write_raw_stdout_line(&summary.to_plain_text())
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextManifestItemSummary {
    pub item_id: String,
    pub category: String,
    pub provenance: String,
    pub source_ref: String,
    pub estimated_tokens: u64,
    pub selected: bool,
    pub mandatory: bool,
    pub pinned: bool,
    pub freshness: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextManifestSummary {
    pub request_id: String,
    pub root_run_id: String,
    pub source_revision: u64,
    pub overlay_revision: u64,
    pub manifest_digest: String,
    pub total_estimated_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch: Option<u64>,
    pub items: Vec<ContextManifestItemSummary>,
}

#[allow(dead_code)]
impl ContextManifestSummary {
    pub fn from_prepared(
        manifest: &davinci_agent::runtime::PreparedContextManifest,
        overlay: Option<&davinci_agent::runtime::ContextOverlay>,
    ) -> Self {
        let items = manifest
            .entries
            .iter()
            .map(|entry| {
                let is_pinned = overlay.is_some_and(|o| o.pinned_ids.contains(&entry.id));
                let is_excluded = overlay.is_some_and(|o| o.excluded_ids.contains(&entry.id));
                let selected = if entry.mandatory {
                    true
                } else if is_excluded {
                    false
                } else if is_pinned {
                    true
                } else {
                    entry.selected
                };

                ContextManifestItemSummary {
                    item_id: entry.id.clone(),
                    category: entry.category.clone(),
                    provenance: entry.provenance_kind.as_str().to_string(),
                    source_ref: entry.source_ref.clone(),
                    estimated_tokens: entry.token_estimate,
                    selected,
                    mandatory: entry.mandatory,
                    pinned: is_pinned,
                    freshness: entry.freshness.clone(),
                }
            })
            .collect();

        Self {
            request_id: manifest.request_id.clone(),
            root_run_id: manifest.root_run_id.to_string(),
            source_revision: manifest.source_revision,
            overlay_revision: manifest.overlay_revision,
            manifest_digest: manifest.manifest_digest.clone(),
            total_estimated_tokens: manifest.estimated_total_tokens,
            context_root_id: manifest.context_root_id.clone(),
            context_epoch: manifest.context_epoch,
            items,
        }
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Context VM state for `/status` and RPC `get_session_stats`. Read-only:
/// building it never compiles an image or touches the VM's metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextVmStatusSummary {
    pub mode: String,
    pub epoch: u64,
    pub checkpoint_id: Option<String>,
    pub delta_count: usize,
    pub episode_count: usize,
    pub hot_event_count: usize,
    pub last_fold_reason: Option<String>,
    pub folds: u64,
    pub page_fault_hits: u64,
    pub page_fault_misses: u64,
    pub prefix_digest: Option<String>,
    pub shadow_missing_user_refs: u64,
    pub shadow_missing_tool_refs: u64,
    pub retrieval_offered: bool,
    pub failure_count: u64,
    pub recent_failures: Vec<String>,
}

impl ContextVmStatusSummary {
    /// `None` while the VM is `off`, so legacy status output is unchanged.
    pub fn for_status(agent: &davinci_agent::Agent) -> Option<Self> {
        (agent.context_vm_mode() != davinci_agent::runtime::ContextVmMode::Off)
            .then(|| Self::from_agent(agent))
    }

    pub fn from_agent(agent: &davinci_agent::Agent) -> Self {
        let mode = match agent.context_vm_mode() {
            davinci_agent::runtime::ContextVmMode::Off => "off",
            davinci_agent::runtime::ContextVmMode::Shadow => "shadow",
            davinci_agent::runtime::ContextVmMode::Active => "active",
        }
        .to_string();
        let Some(runtime) = agent.runtime.as_ref() else {
            return Self {
                mode,
                epoch: 0,
                checkpoint_id: None,
                delta_count: 0,
                episode_count: 0,
                hot_event_count: 0,
                last_fold_reason: None,
                folds: 0,
                page_fault_hits: 0,
                page_fault_misses: 0,
                prefix_digest: None,
                shadow_missing_user_refs: 0,
                shadow_missing_tool_refs: 0,
                retrieval_offered: false,
                failure_count: 0,
                recent_failures: Vec::new(),
            };
        };
        let vm = &runtime.context_vm;
        let root = vm.root();
        let metrics = vm.metrics();
        Self {
            mode,
            epoch: root.epoch,
            checkpoint_id: root.checkpoint.map(|page| page.id),
            delta_count: root.deltas.len(),
            episode_count: root.episodes.len(),
            hot_event_count: root.hot_event_refs.len(),
            last_fold_reason: vm.last_fold_reason(),
            folds: metrics.folds,
            page_fault_hits: metrics.page_fault_hits,
            page_fault_misses: metrics.page_fault_misses,
            prefix_digest: vm
                .prefix_digest()
                .map(|digest| digest.chars().take(12).collect()),
            shadow_missing_user_refs: metrics.shadow_missing_user_refs,
            shadow_missing_tool_refs: metrics.shadow_missing_tool_refs,
            retrieval_offered: agent.context_vm_offers_retrieval(),
            failure_count: vm.failure_count(),
            recent_failures: vm
                .recent_failures()
                .iter()
                .map(|failure| failure.render())
                .collect(),
        }
    }

    /// One `/status` line; the davinci shell renders it as a list item.
    pub fn status_line(&self) -> String {
        let checkpoint = self
            .checkpoint_id
            .as_deref()
            .map(|id| {
                id.rsplit(':')
                    .next()
                    .unwrap_or(id)
                    .chars()
                    .take(12)
                    .collect()
            })
            .unwrap_or_else(|| "none".to_string());
        let mut line = format!(
            "context vm: {} · epoch {} · checkpoint {checkpoint} · {} deltas / {} episodes / {} hot · \
             page faults {} hit / {} miss",
            self.mode,
            self.epoch,
            self.delta_count,
            self.episode_count,
            self.hot_event_count,
            self.page_fault_hits,
            self.page_fault_misses,
        );
        if let Some(reason) = &self.last_fold_reason {
            line.push_str(&format!(" · {} folds, last {reason}", self.folds));
        }
        if let Some(digest) = &self.prefix_digest {
            line.push_str(&format!(" · prefix {digest}"));
        }
        if self.mode == "shadow" {
            line.push_str(&format!(
                " · shadow missing {} user / {} tool refs",
                self.shadow_missing_user_refs, self.shadow_missing_tool_refs
            ));
        }
        if self.failure_count > 0 {
            line.push_str(&format!(" · {} failures", self.failure_count));
            if let Some(last) = self.recent_failures.last() {
                line.push_str(&format!(", last {last}"));
            }
        }
        line
    }

    #[allow(dead_code)]
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredGraphTaskStatus {
    pub id: String,
    pub role: String,
    pub status: String,
    pub attempts: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredGraphStatus {
    pub run_id: String,
    pub goal: String,
    pub phase: String,
    pub lifecycle: String,
    pub revision: u64,
    pub cost_usd: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub elapsed_ms: u64,
    pub tasks: Vec<StructuredGraphTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_blocker: Option<String>,
}

#[allow(dead_code)]
impl StructuredGraphStatus {
    pub fn from_graph_run(run: &crate::native_extensions::graph::GraphRun) -> Self {
        let now = crate::native_extensions::graph::graph_now_ms();
        let elapsed_ms = now.saturating_sub(run.counters.started_at);
        let tasks = run
            .tasks
            .iter()
            .map(|t| {
                let dur = match (t.started_at, t.ended_at) {
                    (Some(s), Some(e)) => e.saturating_sub(s),
                    (Some(s), None) => now.saturating_sub(s),
                    _ => 0,
                };
                StructuredGraphTaskStatus {
                    id: t.id.clone(),
                    role: t.role.as_str().to_string(),
                    status: t.status.as_str().to_string(),
                    attempts: t.attempts,
                    input_tokens: t.usage.input,
                    output_tokens: t.usage.output,
                    duration_ms: dur,
                    depends_on: t.depends_on.clone(),
                    error: t.error.clone(),
                }
            })
            .collect();

        Self {
            run_id: run.run_id.clone(),
            goal: run.goal.clone(),
            phase: run.phase.as_str().to_string(),
            lifecycle: run.current_lifecycle().as_str().to_string(),
            revision: run.revision,
            cost_usd: run.counters.cost_usd,
            total_input_tokens: run.total_input(),
            total_output_tokens: run.total_output(),
            elapsed_ms,
            tasks,
            blocked_reason: run.blocked_reason.clone(),
            active_blocker: None,
        }
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "GRAPH · {} [{}] (lifecycle: {})\n",
            self.goal, self.phase, self.lifecycle
        ));
        out.push_str(&format!(
            "Run ID: {} · Revision: {} · Cost: ${:.4}\n",
            self.run_id, self.revision, self.cost_usd
        ));
        out.push_str("Tasks:\n");
        for task in &self.tasks {
            out.push_str(&format!(
                "  - {} ({}): {} [attempt {}]\n",
                task.id, task.role, task.status, task.attempts
            ));
            if !task.depends_on.is_empty() {
                out.push_str(&format!("    depends on: {}\n", task.depends_on.join(", ")));
            }
            if let Some(ref err) = task.error {
                out.push_str(&format!("    error: {err}\n"));
            }
        }
        out
    }
}

#[allow(dead_code)]
pub fn print_graph_status_json(status: &StructuredGraphStatus) -> io::Result<()> {
    let json = status
        .to_json_string()
        .map_err(|e| io::Error::other(e.to_string()))?;
    write_raw_stdout_line(&json)
}

#[allow(dead_code)]
pub fn print_graph_status_plain(status: &StructuredGraphStatus) -> io::Result<()> {
    write_raw_stdout_line(&status.to_plain_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftover_temp_files(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn atomic_write_creates_and_replaces_the_file_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last.txt");
        write_file_atomically(&path, "first reply").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first reply");
        write_file_atomically(&path, "").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        write_file_atomically(&path, "héllo\nsecond").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "héllo\nsecond");
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn atomic_write_failure_reports_the_error_and_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        // The parent directory does not exist: the temp file cannot be made.
        let missing = dir.path().join("no-such-dir").join("last.txt");
        let error = write_file_atomically(&missing, "reply").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(!missing.exists());

        // The target is a directory: the rename fails after the temp file was
        // written, the directory survives and the temp file is removed.
        let occupied = dir.path().join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), "kept").unwrap();
        assert!(write_file_atomically(&occupied, "reply").is_err());
        assert_eq!(
            std::fs::read_to_string(occupied.join("keep.txt")).unwrap(),
            "kept"
        );
        assert!(leftover_temp_files(dir.path()).is_empty());

        let no_name = write_file_atomically(std::path::Path::new(".."), "reply").unwrap_err();
        assert_eq!(no_name.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn backpressure_detects_enobufs_and_eagain() {
        let again = io::Error::from_raw_os_error(11);
        let nobufs = io::Error::from_raw_os_error(55);
        assert!(is_stdout_backpressure(&again));
        assert!(is_stdout_backpressure(&nobufs));
        assert!(!is_stdout_backpressure(&io::Error::new(
            ErrorKind::BrokenPipe,
            "pipe"
        )));
    }

    #[test]
    fn test_completion_json_stdout_valid() {
        let summary = OutputCompletionSummary {
            task_id: "task-json-1".into(),
            allowed: true,
            source_fingerprint: "sha256:abc123456789".into(),
            evaluated_at_ms: 1700000000000,
            dimensions: vec![OutputDimensionSummary {
                dimension: "implementation".into(),
                state: "passed_current".into(),
                evidence_label: "passed on current source".into(),
                required: true,
                proof: Some("write crates/davinci-coding-agent/src/output.rs".into()),
            }],
            remaining_gaps: vec![],
        };

        let json_str = summary.to_json_string().unwrap();
        assert!(json_str.contains("\"task_id\": \"task-json-1\""));
        assert!(json_str.contains("\"allowed\": true"));
        assert!(json_str.contains("\"source_fingerprint\": \"sha256:abc123456789\""));

        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["task_id"], "task-json-1");
        assert_eq!(parsed["allowed"], true);
    }

    #[test]
    fn test_completion_plain_text_format() {
        let summary = OutputCompletionSummary {
            task_id: "task-plain-1".into(),
            allowed: false,
            source_fingerprint: "sha256:xyz987654".into(),
            evaluated_at_ms: 1700000000000,
            dimensions: vec![
                OutputDimensionSummary {
                    dimension: "implementation".into(),
                    state: "passed_current".into(),
                    evidence_label: "passed on current source".into(),
                    required: true,
                    proof: Some("implementation tool".into()),
                },
                OutputDimensionSummary {
                    dimension: "build".into(),
                    state: "failed".into(),
                    evidence_label: "failed".into(),
                    required: true,
                    proof: None,
                },
            ],
            remaining_gaps: vec!["Build failed with compiler error".into()],
        };

        let plain = summary.to_plain_text();
        assert!(plain.contains("Completion Status: INCOMPLETE"));
        assert!(plain.contains("Source Fingerprint: sha256:xyz987654"));
        assert!(plain.contains("build: failed (required)"));
        assert!(plain.contains("Remaining Gaps:"));
        assert!(plain.contains("! Build failed with compiler error"));
    }

    #[test]
    fn test_structured_graph_status_formatting() {
        let status = StructuredGraphStatus {
            run_id: "run-test-123".into(),
            goal: "Implement OAuth refresh".into(),
            phase: "implement".into(),
            lifecycle: "running".into(),
            revision: 3,
            cost_usd: 0.125,
            total_input_tokens: 25000,
            total_output_tokens: 3500,
            elapsed_ms: 45000,
            tasks: vec![StructuredGraphTaskStatus {
                id: "plan-1".into(),
                role: "planner".into(),
                status: "succeeded".into(),
                attempts: 1,
                input_tokens: 10000,
                output_tokens: 1500,
                duration_ms: 12000,
                depends_on: vec!["classify".into()],
                error: None,
            }],
            blocked_reason: None,
            active_blocker: None,
        };

        let json_str = status.to_json_string().unwrap();
        assert!(json_str.contains("\"runId\": \"run-test-123\""));
        assert!(json_str.contains("\"lifecycle\": \"running\""));
        assert!(json_str.contains("\"revision\": 3"));

        let plain = status.to_plain_text();
        assert!(plain.contains("GRAPH · Implement OAuth refresh [implement] (lifecycle: running)"));
        assert!(plain.contains("plan-1 (planner): succeeded [attempt 1]"));
    }
}
