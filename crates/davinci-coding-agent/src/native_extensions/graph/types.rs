//! Shared types for the graph-engineer runtime.
//!
//! The conversation transcript is communication; these types are execution
//! truth. Artifacts are the ONLY data that crosses node boundaries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        #[allow(dead_code)] // one generated helper set per enum; not every enum needs all of it
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($text => Some($name::$variant),)+
                    _ => None,
                }
            }

            pub fn names() -> String {
                Self::ALL
                    .iter()
                    .map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_enum!(TaskClass {
    Trivial => "trivial",
    Bug => "bug",
    Feature => "feature",
    Refactor => "refactor",
    Investigation => "investigation",
});

string_enum!(Complexity {
    Trivial => "trivial",
    Standard => "standard",
    Complex => "complex",
});

string_enum!(ResearchKind {
    CodeSearch => "code_search",
    TestBaseline => "test_baseline",
    History => "history",
    Docs => "docs",
});

string_enum!(Role {
    Classifier => "classifier",
    Researcher => "researcher",
    TestAnalyzer => "test-analyzer",
    Historian => "historian",
    Planner => "planner",
    Writer => "writer",
    Reviewer => "reviewer",
});

string_enum!(ArtifactKind {
    Classification => "classification",
    Evidence => "evidence",
    Plan => "plan",
    PatchReport => "patch-report",
    Review => "review",
});

string_enum!(Phase {
    Classify => "classify",
    Investigate => "investigate",
    Plan => "plan",
    Implement => "implement",
    Verify => "verify",
    Review => "review",
    Done => "done",
    Blocked => "blocked",
    Cancelled => "cancelled",
});

string_enum!(TaskStatus {
    Pending => "pending",
    Ready => "ready",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});

string_enum!(BashPolicy {
    None => "none",
    ReadOnly => "read-only",
    ReadAndTest => "read-and-test",
    WriteNoGitMutation => "write-no-git-mutation",
});

string_enum!(Confidence {
    High => "high",
    Medium => "medium",
    Low => "low",
});

string_enum!(Severity {
    Blocker => "blocker",
    Major => "major",
    Minor => "minor",
});

string_enum!(Verdict {
    Approve => "approve",
    ChangesRequired => "changes_required",
});

/// Per-role worker timeouts. A value of 0 means "no timeout".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoleTimeouts(BTreeMap<String, u64>);

impl RoleTimeouts {
    /// Every role starts unlimited; a project may opt back into deadlines.
    pub fn unlimited() -> Self {
        Self(
            Role::ALL
                .iter()
                .map(|role| (role.as_str().to_string(), 0))
                .collect(),
        )
    }

    pub fn get(&self, role: Role) -> u64 {
        self.0.get(role.as_str()).copied().unwrap_or(0)
    }

    pub fn set(&mut self, role: Role, timeout_ms: u64) {
        self.0.insert(role.as_str().to_string(), timeout_ms);
    }
}

impl Default for RoleTimeouts {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Spend and shape limits. Every field that bounds time or money defaults to
/// `0`, which means unlimited: a run stops when it finishes, blocks, or the
/// operator aborts it. `max_revision_cycles` and `max_replans` stay positive
/// because they bound loops, not spend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphBudgets {
    pub max_researchers: u32,
    pub max_parallel_workers: u32,
    /// 0 = unlimited.
    pub max_workers: u32,
    pub max_revision_cycles: u32,
    pub max_replans: u32,
    /// 0 = unlimited.
    pub max_cost_usd: f64,
    /// 0 = no deadline.
    pub run_deadline_ms: u64,
    /// 0 = no timeout, per role.
    pub worker_timeout_ms: RoleTimeouts,
    /// 0 = no timeout for a single verification command.
    pub verify_command_timeout_ms: u64,
}

impl Default for GraphBudgets {
    fn default() -> Self {
        Self {
            max_researchers: 3,
            max_parallel_workers: 3,
            max_workers: 0,
            max_revision_cycles: 3,
            max_replans: 2,
            max_cost_usd: 0.0,
            run_deadline_ms: 0,
            worker_timeout_ms: RoleTimeouts::unlimited(),
            verify_command_timeout_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub turns: u64,
}

impl WorkerUsage {
    pub fn delta(current: &WorkerUsage, previous: &WorkerUsage) -> WorkerUsage {
        WorkerUsage {
            input: current.input.saturating_sub(previous.input),
            output: current.output.saturating_sub(previous.output),
            cache_read: current.cache_read.saturating_sub(previous.cache_read),
            cache_write: current.cache_write.saturating_sub(previous.cache_write),
            cost_usd: (current.cost_usd - previous.cost_usd).max(0.0),
            turns: current.turns.saturating_sub(previous.turns),
        }
    }

    pub fn add(&mut self, other: &WorkerUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost_usd += other.cost_usd;
        self.turns += other.turns;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTaskState {
    /// e.g. "classify", "research-1", "plan-1", "implement-2", "review-1"
    pub id: String,
    pub role: Role,
    pub expect: ArtifactKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub attempts: u32,
    /// Relative to the run dir, e.g. "artifacts/research-1.json".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: WorkerUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Live view: last tool call or turn reported by the worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
}

impl GraphTaskState {
    pub fn new(
        id: impl Into<String>,
        role: Role,
        expect: ArtifactKind,
        depends_on: Vec<String>,
        focus: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            role,
            expect,
            depends_on,
            focus,
            status: TaskStatus::Pending,
            attempts: 0,
            artifact_file: None,
            error: None,
            usage: WorkerUsage::default(),
            started_at: None,
            ended_at: None,
            last_activity: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphCounters {
    pub workers_spawned: u32,
    pub revision_cycles: u32,
    pub replans: u32,
    pub cost_usd: f64,
    pub started_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRun {
    pub version: u32,
    pub run_id: String,
    pub goal: String,
    pub cwd: String,
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced: Option<Complexity>,
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<Classification>,
    /// Present only when the run was decomposed into 2+ milestones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Vec<String>>,
    /// 1-based index of the milestone currently being delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_milestone: Option<usize>,
    #[serde(default)]
    pub tasks: Vec<GraphTaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationResult>,
    pub budgets: GraphBudgets,
    pub counters: GraphCounters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    pub updated_at: u64,
}

impl GraphRun {
    pub fn task(&self, id: &str) -> Option<&GraphTaskState> {
        self.tasks.iter().find(|task| task.id == id)
    }

    /// A node may only start once every node it declares a dependency on has
    /// succeeded. The pipeline builds those edges itself, so a violation means
    /// a controller bug, not a model mistake.
    pub fn unmet_dependencies(&self, task: &GraphTaskState) -> Vec<String> {
        task.depends_on
            .iter()
            .filter(|dependency| {
                self.task(dependency)
                    .map(|dependency| dependency.status != TaskStatus::Succeeded)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    pub fn total_input(&self) -> u64 {
        self.tasks.iter().map(|task| task.usage.input).sum()
    }

    pub fn total_output(&self) -> u64 {
        self.tasks.iter().map(|task| task.usage.output).sum()
    }
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchRequest {
    pub kind: ResearchKind,
    pub focus: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub task_class: TaskClass,
    pub complexity: Complexity,
    pub rationale: String,
    #[serde(default)]
    pub research_tasks: Vec<ResearchRequest>,
    /// Ordered, independently deliverable sub-goals. Empty/absent = single
    /// deliverable; each milestone gets its own plan/implement/verify/review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceFinding {
    pub claim: String,
    /// "path/to/file.rs:123" style references.
    pub refs: Vec<String>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestBaseline {
    pub command: String,
    pub exit_code: i64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceArtifact {
    pub kind: ResearchKind,
    #[serde(default)]
    pub findings: Vec<EvidenceFinding>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub gaps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_baseline: Option<TestBaseline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTest {
    pub file: String,
    pub behavior: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImplementationPlan {
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub tests_to_add: Vec<PlanTest>,
    /// Exact runnable shell commands.
    #[serde(default)]
    pub tests_to_run: Vec<String>,
    pub completion_criteria: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchReport {
    #[serde(default)]
    pub changed_files: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub deviations: Vec<String>,
    pub plan_invalidated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewIssue {
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub verdict: Verdict,
    #[serde(default)]
    pub issues: Vec<ReviewIssue>,
    #[serde(default)]
    pub notes: String,
}

/// A validated node output. The variant is chosen by the node's `expect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Artifact {
    Classification(Classification),
    Evidence(Box<EvidenceArtifact>),
    Plan(Box<ImplementationPlan>),
    PatchReport(Box<PatchReport>),
    Review(Box<ReviewDecision>),
}

impl Artifact {
    pub fn as_classification(&self) -> Option<&Classification> {
        match self {
            Artifact::Classification(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_evidence(&self) -> Option<&EvidenceArtifact> {
        match self {
            Artifact::Evidence(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_plan(&self) -> Option<&ImplementationPlan> {
        match self {
            Artifact::Plan(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_patch_report(&self) -> Option<&PatchReport> {
        match self {
            Artifact::PatchReport(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_review(&self) -> Option<&ReviewDecision> {
        match self {
            Artifact::Review(value) => Some(value),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCommandSpec {
    pub name: String,
    pub command: String,
    /// True when the command came from the plan's testsToRun, not config.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_plan: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCommandResult {
    pub name: String,
    pub command: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub output_tail: String,
    /// Plan-invented command that does not exist; excluded from pass/fail.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    pub commands: Vec<VerificationCommandResult>,
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Workers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub task_id: String,
    pub role: Role,
    pub expect: ArtifactKind,
    /// User-message content (markdown).
    pub briefing: String,
    /// Appended role system prompt.
    pub system_prompt: String,
    pub cwd: std::path::PathBuf,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    /// Full --tools allowlist; ALWAYS includes "graph_submit".
    pub tools: Vec<String>,
    /// Extra extension paths passed to the child via -e.
    pub extra_extensions: Vec<String>,
    /// 0 = no timeout.
    pub timeout_ms: u64,
    pub artifact_path: std::path::PathBuf,
    /// Human-readable live transcript the runner appends to.
    pub transcript_path: Option<std::path::PathBuf>,
    pub project_trusted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WorkerResult {
    /// Process exited 0 AND a valid artifact file exists.
    pub ok: bool,
    pub exit_code: i32,
    pub artifact: Option<Artifact>,
    /// Last assistant text (diagnostics).
    pub final_text: String,
    pub stderr: String,
    pub usage: WorkerUsage,
    pub timed_out: bool,
    /// Structured reason for a failed child, when the process did not explain it.
    pub failure_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_budget_that_bounds_time_or_money_defaults_to_unlimited() {
        let budgets = GraphBudgets::default();
        assert_eq!(budgets.max_cost_usd, 0.0);
        assert_eq!(budgets.run_deadline_ms, 0);
        assert_eq!(budgets.max_workers, 0);
        assert_eq!(budgets.verify_command_timeout_ms, 0);
        for role in Role::ALL {
            assert_eq!(budgets.worker_timeout_ms.get(*role), 0, "{role}");
        }
        // Loop bounds are not spend bounds; they stay on.
        assert!(budgets.max_revision_cycles > 0);
        assert!(budgets.max_replans > 0);
    }

    #[test]
    fn string_enums_round_trip_through_their_wire_names() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(*role));
        }
        assert_eq!(Role::parse("nope"), None);
        assert_eq!(ArtifactKind::PatchReport.as_str(), "patch-report");
    }

    #[test]
    fn a_node_waits_for_every_dependency() {
        let mut run = GraphRun {
            version: 1,
            run_id: "r1".into(),
            goal: "g".into(),
            cwd: ".".into(),
            phase: Phase::Investigate,
            forced: None,
            dry_run: false,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![
                GraphTaskState::new("a", Role::Researcher, ArtifactKind::Evidence, vec![], None),
                GraphTaskState::new("b", Role::Researcher, ArtifactKind::Evidence, vec![], None),
                GraphTaskState::new(
                    "c",
                    Role::Planner,
                    ArtifactKind::Plan,
                    vec!["a".into(), "b".into()],
                    None,
                ),
            ],
            verification: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            updated_at: 0,
        };
        let planner = run.tasks[2].clone();
        assert_eq!(run.unmet_dependencies(&planner), vec!["a", "b"]);
        run.tasks[0].status = TaskStatus::Succeeded;
        assert_eq!(run.unmet_dependencies(&planner), vec!["b"]);
        run.tasks[1].status = TaskStatus::Succeeded;
        assert!(run.unmet_dependencies(&planner).is_empty());
        // A dependency the run never created counts as unmet, not as satisfied.
        let orphan = GraphTaskState::new(
            "d",
            Role::Writer,
            ArtifactKind::PatchReport,
            vec!["missing".into()],
            None,
        );
        assert_eq!(run.unmet_dependencies(&orphan), vec!["missing"]);
    }
}
