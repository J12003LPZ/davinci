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
                out.push_str(&format!("    Proof: {}\n", p));
            }
        }
        if !self.remaining_gaps.is_empty() {
            out.push_str("Remaining Gaps:\n");
            for gap in &self.remaining_gaps {
                out.push_str(&format!("  ! {}\n", gap));
            }
        }
        out
    }
}

#[allow(dead_code)]
pub fn print_completion_json(summary: &OutputCompletionSummary) -> io::Result<()> {
    let json = summary
        .to_json_string()
        .map_err(|e| io::Error::new(ErrorKind::Other, e.to_string()))?;
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
                let is_pinned = overlay.map_or(false, |o| o.pinned_ids.contains(&entry.id));
                let is_excluded = overlay.map_or(false, |o| o.excluded_ids.contains(&entry.id));
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

#[allow(dead_code)]
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
    pub page_fault_hits: u64,
    pub page_fault_misses: u64,
    pub prefix_digest: Option<String>,
}

#[allow(dead_code)]
impl ContextVmStatusSummary {
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
                page_fault_hits: 0,
                page_fault_misses: 0,
                prefix_digest: None,
            };
        };
        let root = runtime.context_vm.root();
        let metrics = runtime.context_vm.metrics();
        Self {
            mode,
            epoch: root.epoch,
            checkpoint_id: root.checkpoint.map(|page| page.id),
            delta_count: root.deltas.len(),
            episode_count: root.episodes.len(),
            hot_event_count: root.hot_event_refs.len(),
            last_fold_reason: runtime.context_vm.last_fold_reason(),
            page_fault_hits: metrics.page_fault_hits,
            page_fault_misses: metrics.page_fault_misses,
            prefix_digest: runtime
                .context_vm
                .prefix_digest()
                .map(|digest| digest.chars().take(12).collect()),
        }
    }

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
                out.push_str(&format!("    error: {}\n", err));
            }
        }
        out
    }
}

#[allow(dead_code)]
pub fn print_graph_status_json(status: &StructuredGraphStatus) -> io::Result<()> {
    let json = status
        .to_json_string()
        .map_err(|e| io::Error::new(ErrorKind::Other, e.to_string()))?;
    write_raw_stdout_line(&json)
}

#[allow(dead_code)]
pub fn print_graph_status_plain(status: &StructuredGraphStatus) -> io::Result<()> {
    write_raw_stdout_line(&status.to_plain_text())
}

#[cfg(test)]
mod tests {
    use super::*;

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
