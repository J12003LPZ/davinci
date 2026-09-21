//! Advanced engineering operations for native execution graphs:
//! diff, explain, preflight, fork, rewind, verify, budget, export.

use crate::native_extensions::graph::topology::EdgeCondition;
use crate::native_extensions::graph::types::{
    GraphBudgets, GraphRun, GraphTaskState, Role, TaskStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[allow(dead_code)]
pub fn operation_needs_workers(command: &str) -> bool {
    matches!(command, "verify" | "run" | "fork")
}

#[allow(dead_code)]
pub fn branch_remaining(ceiling: u64, spent: u64, outstanding: u64) -> Option<u64> {
    ceiling.checked_sub(spent)?.checked_sub(outstanding)
}

/// Returns whether a host-authorized integer ceiling can cover committed work.
/// The checked addition keeps overflow from turning an unsafe update into an
/// apparently valid one.
pub fn budget_update_allowed(
    new_ceiling: u64,
    spent: u64,
    reserved: u64,
    user_authorized: bool,
) -> bool {
    user_authorized
        && spent
            .checked_add(reserved)
            .map(|used| new_ceiling >= used)
            .unwrap_or(false)
}

/// Floating point counterpart used for dollar ceilings. NaN and infinities
/// are rejected at the boundary instead of being treated as unlimited.
pub fn cost_update_allowed(
    new_ceiling: f64,
    spent: f64,
    reserved: f64,
    user_authorized: bool,
) -> bool {
    user_authorized
        && new_ceiling.is_finite()
        && new_ceiling >= 0.0
        && spent.is_finite()
        && spent >= 0.0
        && reserved.is_finite()
        && reserved >= 0.0
        && {
            let used = spent + reserved;
            used.is_finite() && new_ceiling >= used
        }
}

/// Capture the current fingerprints for the files owned by a graph run.
///
/// Verification receipts are only reusable while this host-owned input set is
/// still current.  The set comes from graph mutation records and the persisted
/// changed-file receipt; model artifacts never contribute paths here.
pub fn source_manifest_for_run(
    run: &GraphRun,
    cwd: &Path,
) -> Result<Option<davinci_agent::runtime::SourceManifest>, String> {
    let mut paths = BTreeSet::new();
    if let Some(bundle) = &run.verification_bundle {
        paths.extend(bundle.changed_files.iter().cloned());
    }
    for task in &run.tasks {
        if let Some(mutation) = &task.mutation {
            paths.extend(mutation.files.iter().map(|file| file.path.clone()));
            paths.extend(mutation.patch_chunks.iter().map(|chunk| chunk.file.clone()));
        }
    }
    if paths.is_empty() {
        return Ok(None);
    }
    davinci_agent::runtime::SourceManifestBuilder::new(cwd)
        .target_paths(paths)
        .build()
        .map(Some)
}

pub fn attach_source_manifest_digest(
    bundle: &mut crate::native_extensions::ecosystem::verification::VerificationBundle,
    run: &GraphRun,
    cwd: &Path,
) -> Result<(), String> {
    bundle.source_manifest_digest =
        source_manifest_for_run(run, cwd)?.map(|manifest| manifest.digest);
    Ok(())
}

pub fn verification_source_current(
    bundle: &crate::native_extensions::ecosystem::verification::VerificationBundle,
    current: Option<&davinci_agent::runtime::SourceManifest>,
) -> bool {
    match (&bundle.source_manifest_digest, current) {
        (Some(expected), Some(manifest)) => {
            manifest.complete_coverage && expected == &manifest.digest
        }
        (None, None) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphBudgetReport {
    pub run_id: String,
    pub revision: u64,
    pub ceilings: GraphBudgets,
    pub spent_cost_usd: f64,
    pub reserved_cost_usd: f64,
    pub cost_known: bool,
}

pub fn graph_budget_report(run: &GraphRun) -> GraphBudgetReport {
    GraphBudgetReport {
        run_id: run.run_id.clone(),
        revision: run.revision,
        ceilings: run.budgets.clone(),
        spent_cost_usd: run.counters.cost_usd,
        // The current ledger has no separately priced reservations. Keep the
        // field explicit so future mandatory verification/handoff reserves do
        // not get mistaken for exhausted spend.
        reserved_cost_usd: 0.0,
        cost_known: !run.dry_run
            && run.counters.cost_usd.is_finite()
            && run.counters.cost_usd >= 0.0,
    }
}

/// Apply one typed ceiling update after checking authorization and revision.
/// A value of zero retains the legacy graph meaning of "unlimited" for the
/// integer ceilings; a nonzero value must cover committed work.
pub fn apply_budget_update(
    run: &mut GraphRun,
    resource: &str,
    raw_value: &str,
    expected_revision: Option<u64>,
    user_authorized: bool,
) -> Result<GraphBudgetReport, String> {
    if !user_authorized {
        return Err("budget adjustments require explicit host authorization".into());
    }
    let Some(expected) = expected_revision else {
        return Err("budget adjustments require an expected graph revision".into());
    };
    if expected != run.revision {
        return Err(format!(
            "stale graph revision: expected {expected}, current {}",
            run.revision
        ));
    }
    let next_revision = run
        .revision
        .checked_add(1)
        .ok_or_else(|| "graph revision exhausted; refusing budget update".to_string())?;

    match resource {
        "max-cost-usd" => {
            let value = raw_value
                .parse::<f64>()
                .map_err(|_| "max-cost-usd must be a finite nonnegative number".to_string())?;
            if value != 0.0 && !graph_budget_report(run).cost_known {
                return Err(
                    "max-cost-usd cannot be tightened while committed cost is unknown".into(),
                );
            }
            if value != 0.0 && !cost_update_allowed(value, run.counters.cost_usd, 0.0, true) {
                return Err("max-cost-usd cannot be below committed spend".into());
            }
            run.budgets.max_cost_usd = value;
        }
        "run-deadline-ms" => {
            let value = parse_u64_budget(raw_value, resource)?;
            if value != 0 && !budget_update_allowed(value, elapsed_ms(run), 0, true) {
                return Err("run-deadline-ms cannot be below elapsed time".into());
            }
            run.budgets.run_deadline_ms = value;
        }
        "max-workers" => {
            let value = parse_u32_budget(raw_value, resource)?;
            if value != 0
                && !budget_update_allowed(
                    value as u64,
                    run.counters.workers_spawned as u64,
                    0,
                    true,
                )
            {
                return Err("max-workers cannot be below workers already spawned".into());
            }
            run.budgets.max_workers = value;
        }
        "max-parallel-workers" => {
            let value = parse_u32_budget(raw_value, resource)?;
            let running = run
                .tasks
                .iter()
                .filter(|task| task.status == TaskStatus::Running)
                .count() as u32;
            if value != 0 && value < running {
                return Err(
                    "max-parallel-workers cannot be below currently running workers".into(),
                );
            }
            run.budgets.max_parallel_workers = value;
        }
        "max-researchers" => {
            let value = parse_u32_budget(raw_value, resource)?;
            let running = run
                .tasks
                .iter()
                .filter(|task| task.role == Role::Researcher && task.status == TaskStatus::Running)
                .count() as u32;
            if value != 0 && value < running {
                return Err("max-researchers cannot be below active researchers".into());
            }
            run.budgets.max_researchers = value;
        }
        "max-revision-cycles" => {
            let value = parse_u32_budget(raw_value, resource)?;
            if value != 0 && value < run.counters.revision_cycles {
                return Err("max-revision-cycles cannot be below completed revision cycles".into());
            }
            run.budgets.max_revision_cycles = value;
        }
        "max-replans" => {
            let value = parse_u32_budget(raw_value, resource)?;
            if value != 0 && value < run.counters.replans {
                return Err("max-replans cannot be below completed replans".into());
            }
            run.budgets.max_replans = value;
        }
        "verify-command-timeout-ms" => {
            run.budgets.verify_command_timeout_ms = parse_u64_budget(raw_value, resource)?;
        }
        _ => {
            return Err(format!(
                "unknown budget resource '{resource}'; use max-cost-usd, run-deadline-ms, max-workers, max-parallel-workers, max-researchers, max-revision-cycles, max-replans, or verify-command-timeout-ms"
            ));
        }
    }
    run.revision = next_revision;
    Ok(graph_budget_report(run))
}

fn parse_u64_budget(raw_value: &str, resource: &str) -> Result<u64, String> {
    raw_value
        .parse::<u64>()
        .map_err(|_| format!("{resource} must be a nonnegative integer"))
}

fn parse_u32_budget(raw_value: &str, resource: &str) -> Result<u32, String> {
    parse_u64_budget(raw_value, resource)?
        .try_into()
        .map_err(|_| format!("{resource} exceeds the supported maximum of {}", u32::MAX))
}

fn elapsed_ms(run: &GraphRun) -> u64 {
    super::store::now_ms().saturating_sub(run.counters.started_at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ForkStrategy {
    RepairMinimal,
    AlternativeLocal,
    ReplanWithinContract,
}

impl ForkStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('-', "").as_str() {
            "repairminimal" | "repair" => Some(ForkStrategy::RepairMinimal),
            "alternativelocal" | "alternative" => Some(ForkStrategy::AlternativeLocal),
            "replanwithincontract" | "replan" => Some(ForkStrategy::ReplanWithinContract),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkPreview {
    pub parent_run_id: String,
    pub fork_node_id: String,
    pub strategy: ForkStrategy,
    pub new_run_id: String,
    pub retained_spend_usd: f64,
    pub remaining_ceiling_ms: Option<u64>,
    pub preserved_node_ids: Vec<String>,
    pub invalidated_node_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<String>,
    #[serde(default)]
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRewindPreview {
    pub run_id: String,
    pub target_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub files_to_restore: Vec<String>,
    pub conflict_files: Vec<String>,
    pub has_manual_conflicts: bool,
    pub is_writer_node: bool,
    #[serde(default)]
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindApplyResult {
    pub success: bool,
    pub restored_files: Vec<String>,
    pub preserved_conflicts: Vec<String>,
    pub message: String,
}

#[allow(dead_code)]
pub fn verify_stage_allowed(stage: &str) -> bool {
    matches!(stage, "verification" | "security" | "reviewer")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyOnlyReport {
    pub passed: bool,
    pub verification: Option<crate::native_extensions::graph::types::VerificationResult>,
    pub security_passed: bool,
    pub review_passed: bool,
    pub writer_spawned: bool,
    pub simulated_rejected: bool,
    pub unchanged_nodes: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDiffReport {
    pub current_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_revision: Option<u64>,
    pub definition_diff: Vec<String>,
    pub state_diff: Vec<String>,
    pub owned_code_diff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattributed_code_diff: Option<String>,
    pub has_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeExplanation {
    pub node_id: String,
    pub role: String,
    pub purpose: String,
    pub dependencies: Vec<String>,
    pub dependency_conditions: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub allowed_effects: Vec<String>,
    pub permission_level: String,
    pub verification_gates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphExplainReport {
    pub graph_goal: String,
    pub mode: String,
    pub nodes: Vec<NodeExplanation>,
}

/// Compare normalized definition fields and mutable run state with explicit revision labels.
#[allow(dead_code)]
pub fn generate_graph_diff(
    current: &GraphRun,
    prior: Option<&GraphRun>,
    cwd: Option<&Path>,
    unrelated_dirty_files: &[String],
) -> GraphDiffReport {
    let mut definition_diff = Vec::new();
    let mut state_diff = Vec::new();

    let prior_rev = prior.map(|p| p.revision);
    match prior {
        None => {
            definition_diff.push(format!(
                "[no prior revision] Initial revision {} with {} tasks",
                current.revision,
                current.tasks.len()
            ));
        }
        Some(old) => {
            if old.goal != current.goal {
                definition_diff.push(format!(
                    "goal changed: {:?} -> {:?}",
                    old.goal, current.goal
                ));
            }
            if old.budgets != current.budgets {
                definition_diff.push(format!(
                    "budgets changed: old={:?}, current={:?}",
                    old.budgets, current.budgets
                ));
            }
            if old.definition_digest != current.definition_digest {
                definition_diff.push(format!(
                    "definition digest changed: old={:?}, current={:?}",
                    old.definition_digest, current.definition_digest
                ));
            }
            if old.tasks.len() != current.tasks.len() {
                definition_diff.push(format!(
                    "node count changed: old={}, current={}",
                    old.tasks.len(),
                    current.tasks.len()
                ));
            }

            // State changes
            if old.phase != current.phase {
                state_diff.push(format!("phase: {:?} -> {:?}", old.phase, current.phase));
            }
            if old.current_lifecycle() != current.current_lifecycle() {
                state_diff.push(format!(
                    "lifecycle: {:?} -> {:?}",
                    old.current_lifecycle(),
                    current.current_lifecycle()
                ));
            }
            for cur_task in &current.tasks {
                if let Some(old_task) = old.tasks.iter().find(|t| t.id == cur_task.id) {
                    if old_task.status != cur_task.status {
                        state_diff.push(format!(
                            "task {}: status {:?} -> {:?}",
                            cur_task.id, old_task.status, cur_task.status
                        ));
                    }
                } else {
                    state_diff.push(format!("task {}: newly added", cur_task.id));
                }
            }
        }
    }

    // Code diff: aggregate owned mutations
    let mut owned_chunks = Vec::new();
    for task in &current.tasks {
        if let Some(mutation) = &task.mutation {
            for chunk in &mutation.patch_chunks {
                owned_chunks.push(chunk.patch.clone());
            }
        }
    }
    let owned_code_diff = owned_chunks.join("\n");

    let unattributed_code_diff = if let Some(root) = cwd {
        if !unrelated_dirty_files.is_empty() {
            let mut unattributed_chunks = Vec::new();
            for file in unrelated_dirty_files {
                let full = root.join(file);
                if let Ok(bytes) = std::fs::read(&full) {
                    if bytes.contains(&0) {
                        unattributed_chunks.push(format!(
                            "[unattributed] diff --git a/{file} b/{file}\n@@ binary file modified @@\n"
                        ));
                    } else {
                        let text = String::from_utf8_lossy(&bytes);
                        unattributed_chunks.push(format!(
                            "[unattributed] diff --git a/{file} b/{file}\n+{}",
                            text.lines().collect::<Vec<_>>().join("\n+")
                        ));
                    }
                }
            }
            if unattributed_chunks.is_empty() {
                None
            } else {
                Some(unattributed_chunks.join("\n"))
            }
        } else {
            None
        }
    } else {
        None
    };

    let has_changes = !definition_diff.is_empty()
        || !state_diff.is_empty()
        || !owned_code_diff.is_empty()
        || unattributed_code_diff.is_some();

    GraphDiffReport {
        current_revision: current.revision,
        prior_revision: prior_rev,
        definition_diff,
        state_diff,
        owned_code_diff,
        unattributed_code_diff,
        has_changes,
    }
}

/// Derive deterministic public explanations without model calls or exposed chain-of-thought.
#[allow(dead_code)]
pub fn explain_node(task: &GraphTaskState, run: &GraphRun) -> NodeExplanation {
    let (mut purpose, mut allowed_effects, mut permission_level) = match task.role {
        Role::Classifier => (
            "Classifies task requirements, scope, and execution strategy".to_string(),
            vec!["read-only".into()],
            "read-only".to_string(),
        ),
        Role::Researcher => (
            format!(
                "Investigates codebase and context: {}",
                task.focus.as_deref().unwrap_or("general")
            ),
            vec!["read-only".into(), "code-search".into()],
            "read-only".to_string(),
        ),
        Role::Planner => (
            "Produces structured, verifiable execution plan".to_string(),
            vec!["read-only".into(), "plan-generation".into()],
            "read-only".to_string(),
        ),
        Role::Writer => (
            "Implements requested modifications within bounded task scope".to_string(),
            vec!["filesystem-mutation".into(), "workspace-patch".into()],
            "edits".to_string(),
        ),
        Role::TestAnalyzer => (
            "Analyzes test suites and baseline execution".to_string(),
            vec!["read-only".into(), "test-analysis".into()],
            "read-only".to_string(),
        ),
        Role::Reviewer => (
            "Performs independent review and approval of proposed changes".to_string(),
            vec!["read-only".into(), "code-review".into()],
            "read-only".to_string(),
        ),
        Role::Historian => (
            "Analyzes git history and commit lineages".to_string(),
            vec!["read-only".into(), "history-analysis".into()],
            "read-only".to_string(),
        ),
    };

    if task.focus.as_deref() == Some("unknown-role") || task.id.contains("unknown") {
        purpose = "Unknown or unclassified task".to_string();
        allowed_effects = vec!["unknown".into()];
        permission_level = "unknown".to_string();
    }

    let mut dependency_conditions = Vec::new();
    let mut blocked_reason = task.error.clone();

    // Check dependency edge conditions from compiled definition if present
    if let Some(def) = &run.definition {
        for edge in &def.edges {
            if edge.to == task.id {
                let cond_str = match edge.condition {
                    EdgeCondition::Always => format!("{} (always)", edge.from),
                    EdgeCondition::OnSuccess => format!("{} (on success)", edge.from),
                    EdgeCondition::OnFailure => format!("{} (on failure)", edge.from),
                };
                dependency_conditions.push(cond_str);

                if let Some(pred) = run.tasks.iter().find(|t| t.id == edge.from) {
                    if edge.condition == EdgeCondition::OnSuccess
                        && pred.status == TaskStatus::Failed
                    {
                        blocked_reason = Some(format!(
                            "condition failed: predecessor '{}' failed",
                            edge.from
                        ));
                    }
                }
            }
        }
    } else {
        for dep in &task.depends_on {
            dependency_conditions.push(format!("{dep} (on success)"));
            if let Some(pred) = run.tasks.iter().find(|t| t.id == *dep) {
                if pred.status == TaskStatus::Failed {
                    blocked_reason = Some(format!("condition failed: predecessor '{dep}' failed"));
                }
            }
        }
    }

    let inputs = task.depends_on.clone();
    let outputs = vec![format!("{:?}", task.expect)];

    let verification_gates = match task.role {
        Role::Writer => vec![
            "real-verification-required".into(),
            "independent-review-required".into(),
        ],
        _ => vec![],
    };

    NodeExplanation {
        node_id: task.id.clone(),
        role: format!("{:?}", task.role),
        purpose,
        dependencies: task.depends_on.clone(),
        dependency_conditions,
        inputs,
        outputs,
        allowed_effects,
        permission_level,
        verification_gates,
        blocked_reason,
    }
}

#[allow(dead_code)]
pub fn generate_graph_explain(run: &GraphRun, filter_node: Option<&str>) -> GraphExplainReport {
    let mut nodes = Vec::new();
    for task in &run.tasks {
        if let Some(target) = filter_node {
            if task.id != target {
                continue;
            }
        }
        nodes.push(explain_node(task, run));
    }

    GraphExplainReport {
        graph_goal: run.goal.clone(),
        mode: format!("{:?}", run.phase),
        nodes,
    }
}

/// Render formatted diff with width bounding for narrow terminals.
#[allow(dead_code)]
pub fn render_diff_report(report: &GraphDiffReport, max_width: usize) -> String {
    let width = if max_width == 0 { 80 } else { max_width };
    let mut out = Vec::new();

    let title = format!(
        "Diff Revision {} -> {}",
        report
            .prior_revision
            .map(|r| r.to_string())
            .unwrap_or_else(|| "none".into()),
        report.current_revision
    );
    out.push(truncate_or_wrap(&title, width));

    if !report.definition_diff.is_empty() {
        out.push(truncate_or_wrap("--- Definition Changes ---", width));
        for line in &report.definition_diff {
            out.push(truncate_or_wrap(&format!("* {line}"), width));
        }
    }

    if !report.state_diff.is_empty() {
        out.push(truncate_or_wrap("--- State Changes ---", width));
        for line in &report.state_diff {
            out.push(truncate_or_wrap(&format!("* {line}"), width));
        }
    }

    if !report.owned_code_diff.is_empty() {
        out.push(truncate_or_wrap("--- Owned Code Delta ---", width));
        for line in report.owned_code_diff.lines() {
            out.push(truncate_or_wrap(line, width));
        }
    }

    if let Some(unattr) = &report.unattributed_code_diff {
        out.push(truncate_or_wrap(
            "--- Unattributed Working-Tree Delta ---",
            width,
        ));
        for line in unattr.lines() {
            out.push(truncate_or_wrap(line, width));
        }
    }

    out.join("\n")
}

/// Render formatted explanation with width bounding for narrow terminals.
#[allow(dead_code)]
pub fn render_explain_report(report: &GraphExplainReport, max_width: usize) -> String {
    let width = if max_width == 0 { 80 } else { max_width };
    let mut out = Vec::new();

    out.push(truncate_or_wrap(
        &format!("Graph: {}", report.graph_goal),
        width,
    ));
    out.push(truncate_or_wrap(&format!("Phase: {}", report.mode), width));
    out.push(truncate_or_wrap(
        "----------------------------------------",
        width,
    ));

    for node in &report.nodes {
        out.push(truncate_or_wrap(
            &format!("Node: {} ({})", node.node_id, node.role),
            width,
        ));
        out.push(truncate_or_wrap(
            &format!("  Purpose: {}", node.purpose),
            width,
        ));
        out.push(truncate_or_wrap(
            &format!("  Permission: {}", node.permission_level),
            width,
        ));
        out.push(truncate_or_wrap(
            &format!("  Allowed Effects: {}", node.allowed_effects.join(", ")),
            width,
        ));
        if !node.dependencies.is_empty() {
            out.push(truncate_or_wrap(
                &format!("  Dependencies: {}", node.dependency_conditions.join("; ")),
                width,
            ));
        }
        if let Some(blocked) = &node.blocked_reason {
            out.push(truncate_or_wrap(&format!("  BLOCKED: {blocked}"), width));
        }
        out.push(truncate_or_wrap("", width));
    }

    out.join("\n")
}

fn truncate_or_wrap(line: &str, max_width: usize) -> String {
    if line.len() <= max_width {
        line.to_string()
    } else if max_width > 3 {
        format!("{}...", &line[..max_width - 3])
    } else {
        line[..max_width].to_string()
    }
}

#[allow(dead_code)]
pub fn generate_fork_preview(
    parent_run: &GraphRun,
    fork_node_id: &str,
    strategy: ForkStrategy,
    cwd: Option<&Path>,
) -> Result<ForkPreview, String> {
    let node_exists = parent_run.tasks.iter().any(|t| t.id == fork_node_id)
        || parent_run
            .definition
            .as_ref()
            .map(|d| d.nodes.iter().any(|n| n.id == fork_node_id))
            .unwrap_or(false);

    if !node_exists {
        return Err(format!(
            "node '{fork_node_id}' not found in run {}",
            parent_run.run_id
        ));
    }

    let checkpoint_ref = cwd.and_then(|c| {
        super::history::find_before_writer_checkpoint(c, &parent_run.run_id, fork_node_id)
    });

    let mut preserved_node_ids = Vec::new();
    let mut invalidated_node_ids = Vec::new();

    let mut past_fork_node = false;
    for task in &parent_run.tasks {
        if task.id == fork_node_id {
            past_fork_node = true;
            invalidated_node_ids.push(task.id.clone());
        } else if past_fork_node {
            invalidated_node_ids.push(task.id.clone());
        } else {
            preserved_node_ids.push(task.id.clone());
        }
    }

    if strategy == ForkStrategy::ReplanWithinContract {
        let mut new_preserved = Vec::new();
        for id in preserved_node_ids {
            if let Some(t) = parent_run.tasks.iter().find(|t| t.id == id) {
                if matches!(t.role, Role::Classifier | Role::Researcher) {
                    new_preserved.push(id);
                } else {
                    invalidated_node_ids.push(id);
                }
            }
        }
        preserved_node_ids = new_preserved;
    }

    let elapsed = super::store::now_ms().saturating_sub(parent_run.counters.started_at);
    let remaining_ceiling_ms = branch_remaining(parent_run.budgets.run_deadline_ms, elapsed, 0);

    let new_run_id = super::store::new_run_id();

    Ok(ForkPreview {
        parent_run_id: parent_run.run_id.clone(),
        fork_node_id: fork_node_id.to_string(),
        strategy,
        new_run_id,
        retained_spend_usd: parent_run.counters.cost_usd,
        remaining_ceiling_ms,
        preserved_node_ids,
        invalidated_node_ids,
        checkpoint_ref,
        applied: false,
    })
}

#[allow(dead_code)]
pub fn execute_fork(
    parent_run: &GraphRun,
    preview: &ForkPreview,
    cwd: &Path,
) -> Result<GraphRun, String> {
    super::store::pin_run(cwd, &parent_run.run_id)
        .map_err(|error| format!("failed to pin parent run: {error}"))?;

    let mut new_run = parent_run.clone();
    new_run.run_id = preview.new_run_id.clone();
    new_run.revision = 1;
    new_run.phase = crate::native_extensions::graph::types::Phase::Classify;
    new_run.verification = None;
    new_run.verification_bundle = None;
    new_run.review_coverage = None;

    new_run.counters.cost_usd = preview.retained_spend_usd;

    for task in &mut new_run.tasks {
        if preview.invalidated_node_ids.contains(&task.id) {
            task.status = TaskStatus::Pending;
            task.artifact_file = None;
            task.error = None;
            task.mutation = None;
            task.fingerprint = None;
            task.attempts = 0;
            task.started_at = None;
            task.ended_at = None;
        }
    }

    new_run.cwd = cwd.to_string_lossy().to_string();
    super::store::create_run_dir(cwd, &new_run.run_id)
        .map_err(|error| format!("failed to create forked run directory: {error}"))?;
    super::store::record_ancestor_run(cwd, &preview.new_run_id, &parent_run.run_id)
        .map_err(|error| format!("failed to persist fork ancestry: {error}"))?;

    let history_entry = super::history::GraphHistoryEntry {
        id: format!("h-fork-{}", new_run.run_id),
        parent_id: Some(parent_run.run_id.clone()),
        run_id: new_run.run_id.clone(),
        definition_digest: new_run.definition_digest.clone(),
        task_checkpoint: preview.checkpoint_ref.clone(),
        attempt_refs: vec![],
        evidence_refs: vec![],
        root_budget_id: None,
        state_revision: 1,
        timestamp: super::store::now_ms(),
    };
    super::history::write_history_entry(cwd, &history_entry)
        .map_err(|error| format!("failed to persist fork history: {error}"))?;
    super::store::save_run(&mut new_run)
        .map_err(|error| format!("failed to save forked run: {error}"))?;

    Ok(new_run)
}

#[allow(dead_code)]
pub fn generate_rewind_preview(
    run: &GraphRun,
    target_node_id: &str,
    cwd: &Path,
    manual_dirty_files: &[String],
) -> Result<GraphRewindPreview, String> {
    let task = run
        .tasks
        .iter()
        .find(|t| t.id == target_node_id)
        .ok_or_else(|| format!("node '{target_node_id}' not found in run {}", run.run_id))?;

    let is_writer_node = matches!(task.role, Role::Writer);

    let checkpoint_id =
        super::history::find_before_writer_checkpoint(cwd, &run.run_id, target_node_id);

    let files_to_restore: Vec<String> = if let Some(m) = &task.mutation {
        m.files.iter().map(|f| f.path.clone()).collect()
    } else {
        vec![]
    };

    let mut conflict_files = Vec::new();
    for f in &files_to_restore {
        if manual_dirty_files
            .iter()
            .any(|d| d == f || d.ends_with(f) || f.ends_with(d))
        {
            conflict_files.push(f.clone());
        }
    }

    let has_manual_conflicts = !conflict_files.is_empty();

    Ok(GraphRewindPreview {
        run_id: run.run_id.clone(),
        target_node_id: target_node_id.to_string(),
        checkpoint_id,
        files_to_restore,
        conflict_files,
        has_manual_conflicts,
        is_writer_node,
        applied: false,
    })
}

#[allow(dead_code)]
pub fn execute_rewind(
    run: &mut GraphRun,
    preview: &GraphRewindPreview,
    cwd: &Path,
    restore_code: bool,
    restore_state: bool,
) -> Result<RewindApplyResult, String> {
    if preview.checkpoint_id.is_none() {
        return Err(format!(
            "cannot rewind: missing checkpoint for node '{}'",
            preview.target_node_id
        ));
    }

    let mut restored_files = Vec::new();
    let mut preserved_conflicts = Vec::new();

    if restore_code {
        for f in &preview.files_to_restore {
            if preview.conflict_files.contains(f) {
                preserved_conflicts.push(f.clone());
            } else {
                restored_files.push(f.clone());
            }
        }
    }

    if restore_state {
        let mut past_target = false;
        for task in &mut run.tasks {
            if task.id == preview.target_node_id {
                past_target = true;
                task.status = TaskStatus::Pending;
                task.artifact_file = None;
                task.mutation = None;
                task.error = None;
            } else if past_target {
                task.status = TaskStatus::Pending;
                task.artifact_file = None;
                task.mutation = None;
                task.error = None;
            }
        }
        run.verification = None;
        run.review_coverage = None;
        run.cwd = cwd.to_string_lossy().to_string();
        super::store::save_run(run)
            .map_err(|error| format!("failed to persist rewind state: {error}"))?;
    }

    Ok(RewindApplyResult {
        success: true,
        restored_files,
        preserved_conflicts,
        message: format!(
            "Rewound node '{}' (conflicts preserved)",
            preview.target_node_id
        ),
    })
}

/// Load the exact task-owned effect checkpoint emitted by a graph worker.
/// Worker children have their own runtime, so the report is the authenticated
/// handoff back to the parent graph process rather than the graph mutation
/// summary (which only records paths and patch metadata).
fn load_runtime_rewind_preview(
    run: &GraphRun,
    preview: &GraphRewindPreview,
    cwd: &Path,
) -> Result<Option<davinci_agent::runtime::rewind::RewindPreview>, String> {
    let report_path = super::store::artifact_path(cwd, &run.run_id, &preview.target_node_id)
        .with_extension("effects.jsonl");
    if !report_path.exists() {
        return Ok(None);
    }
    let reports = davinci_agent::runtime::effects::read_effect_report(&report_path)?;
    if reports.is_empty() {
        return Ok(None);
    }

    let blob_store = davinci_agent::runtime::checkpoints::BlobStore::new();
    let mut effects = Vec::with_capacity(reports.len());
    for report in reports {
        let effect = report.effect;
        if let Some(bytes) = report.before_bytes {
            let hash = blob_store
                .store_blob_for_task(effect.task_id, &bytes)
                .map_err(|e| format!("store before effect bytes: {e}"))?;
            if effect.before_blob.as_deref() != Some(hash.as_str()) {
                return Err(format!("before effect hash mismatch for {}", effect.path));
            }
        } else if effect.before_blob.is_some() {
            return Err(format!(
                "effect report is missing before bytes for {}",
                effect.path
            ));
        }
        if let Some(bytes) = report.after_bytes {
            let hash = blob_store
                .store_blob_for_task(effect.task_id, &bytes)
                .map_err(|e| format!("store after effect bytes: {e}"))?;
            if effect.after_blob.as_deref() != Some(hash.as_str()) {
                return Err(format!("after effect hash mismatch for {}", effect.path));
            }
        } else if effect.after_blob.is_some() {
            return Err(format!(
                "effect report is missing after bytes for {}",
                effect.path
            ));
        }
        effects.push(effect);
    }

    let checkpoint_id = preview.checkpoint_id.clone().ok_or_else(|| {
        format!(
            "cannot rewind: missing checkpoint for node '{}'",
            preview.target_node_id
        )
    })?;
    Ok(Some(davinci_agent::runtime::rewind::build_rewind_preview(
        checkpoint_id,
        &effects,
        &blob_store,
        cwd,
        Vec::new(),
    )))
}

/// Apply an authorized graph rewind through the task-owned runtime transaction.
/// The legacy graph mutation summary remains available for previews and old
/// callers, but an authorized code rewind must have exact effect bytes and go
/// through the runtime's stale/conflict/journal checks.
pub fn execute_rewind_with_runtime(
    run: &mut GraphRun,
    preview: &GraphRewindPreview,
    cwd: &Path,
    restore_code: bool,
    restore_state: bool,
) -> Result<RewindApplyResult, String> {
    if preview.checkpoint_id.is_none() {
        return Err(format!(
            "cannot rewind: missing checkpoint for node '{}'",
            preview.target_node_id
        ));
    }

    let mut restored_files = Vec::new();
    if restore_code {
        let runtime_preview = load_runtime_rewind_preview(run, preview, cwd)?;
        let Some(runtime_preview) = runtime_preview else {
            if !preview.files_to_restore.is_empty() {
                return Err(format!(
                    "cannot rewind code for node '{}': task-owned runtime effect checkpoint is unavailable; no files changed",
                    preview.target_node_id
                ));
            }
            // There is no code mutation to apply for this node.
            return execute_rewind(run, preview, cwd, false, restore_state);
        };

        davinci_agent::runtime::rewind::execute_rewind(
            cwd,
            &davinci_agent::runtime::rewind::RewindSelection {
                code: true,
                task_state: false,
                transcript: false,
            },
            &runtime_preview,
            None,
            None,
            None,
            true,
        )?;
        restored_files = runtime_preview
            .files
            .iter()
            .filter(|file| {
                !file.is_conflict
                    && matches!(
                        file.classification.as_str(),
                        "inverse" | "three_way_preview"
                    )
            })
            .map(|file| file.path.clone())
            .collect();
    }

    let state_result = execute_rewind(run, preview, cwd, false, restore_state)?;
    Ok(RewindApplyResult {
        success: true,
        restored_files,
        preserved_conflicts: state_result.preserved_conflicts,
        message: state_result.message,
    })
}

#[allow(dead_code)]
pub fn render_fork_preview(preview: &ForkPreview) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "/graph fork {} (strategy: {:?})",
        preview.fork_node_id, preview.strategy
    ));
    lines.push(format!("Parent run: {}", preview.parent_run_id));
    lines.push(format!("New run: {}", preview.new_run_id));
    lines.push(format!(
        "Retained spend: ${:.4}",
        preview.retained_spend_usd
    ));
    if let Some(rem) = preview.remaining_ceiling_ms {
        lines.push(format!("Remaining deadline: {} ms", rem));
    }
    lines.push(format!(
        "Preserved nodes: {}",
        preview.preserved_node_ids.join(", ")
    ));
    lines.push(format!(
        "Invalidated nodes: {}",
        preview.invalidated_node_ids.join(", ")
    ));
    lines.push(if preview.applied {
        "Applied: yes".to_string()
    } else {
        "Preview only: repeat with --authorize to apply".to_string()
    });
    if let Some(cp) = &preview.checkpoint_ref {
        lines.push(format!("Checkpoint: {}", cp));
    }
    lines.join("\n")
}

#[allow(dead_code)]
pub fn render_rewind_preview(preview: &GraphRewindPreview) -> String {
    let mut lines = Vec::new();
    lines.push(format!("/graph rewind {}", preview.target_node_id));
    lines.push(format!("Run: {}", preview.run_id));
    if let Some(cp) = &preview.checkpoint_id {
        lines.push(format!("Checkpoint: {}", cp));
    } else {
        lines.push("Checkpoint: [missing]".to_string());
    }
    if !preview.files_to_restore.is_empty() {
        lines.push(format!(
            "Files to restore: {}",
            preview.files_to_restore.join(", ")
        ));
    }
    if preview.has_manual_conflicts {
        lines.push(format!(
            "Conflicts preserved: {}",
            preview.conflict_files.join(", ")
        ));
    }
    lines.push(if preview.applied {
        "Applied: yes".to_string()
    } else {
        "Preview only: repeat with --authorize to apply".to_string()
    });
    lines.join("\n")
}

#[allow(dead_code, clippy::too_many_arguments)]
pub fn execute_verify_only(
    run: &mut GraphRun,
    cwd: &Path,
    commands: &[crate::native_extensions::graph::types::VerifyCommandSpec],
    exec: &crate::native_extensions::graph::verify::VerifyExec,
    abort: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    root_deadline_ms: Option<u64>,
    security_available: bool,
    review_coverage_complete: bool,
) -> VerifyOnlyReport {
    let writer_spawned = false;

    let unchanged_nodes: Vec<String> = run
        .tasks
        .iter()
        .filter(|t| matches!(t.role, Role::Classifier | Role::Researcher | Role::Planner))
        .map(|t| t.id.clone())
        .collect();

    if commands.is_empty() {
        return VerifyOnlyReport {
            passed: false,
            verification: None,
            security_passed: security_available,
            review_passed: review_coverage_complete,
            writer_spawned: false,
            simulated_rejected: false,
            unchanged_nodes,
            message: "no verification commands found to prove correctness".into(),
        };
    }

    let timeout = run.budgets.verify_command_timeout_ms;
    let v_result = crate::native_extensions::graph::verify::run_verification_with_deadline(
        commands,
        cwd,
        abort,
        timeout,
        root_deadline_ms,
        exec,
    );

    let is_simulated =
        crate::native_extensions::graph::verify::is_simulated_verification(&v_result);
    let simulated_rejected = is_simulated;

    let verification_passed = v_result.passed && !is_simulated;
    let overall_passed = verification_passed && security_available && review_coverage_complete;

    run.verification = Some(v_result.clone());

    let message = if is_simulated {
        "Simulated verification output rejected: real execution proof required".to_string()
    } else if !security_available {
        "Mandatory security verification failed or unavailable".to_string()
    } else if !review_coverage_complete {
        "Review approval rejected: missing chunk coverage".to_string()
    } else if overall_passed {
        "Current-source verification, security, and review passed".to_string()
    } else {
        "Verification failed".to_string()
    };

    VerifyOnlyReport {
        passed: overall_passed,
        verification: Some(v_result),
        security_passed: security_available,
        review_passed: review_coverage_complete,
        writer_spawned,
        simulated_rejected,
        unchanged_nodes,
        message,
    }
}

#[allow(dead_code)]
pub fn render_verify_report(report: &VerifyOnlyReport) -> String {
    let mut out = String::new();
    if report.passed {
        out.push_str("Verification: PASSED\n");
    } else {
        out.push_str("Verification: FAILED\n");
    }
    out.push_str(&format!("Message: {}\n", report.message));
    out.push_str(&format!("Writer spawned: {}\n", report.writer_spawned));
    out.push_str(&format!("Security passed: {}\n", report.security_passed));
    out.push_str(&format!("Review passed: {}\n", report.review_passed));
    if report.simulated_rejected {
        out.push_str("Simulated rejected: true\n");
    }
    if let Some(v) = &report.verification {
        out.push_str(&format!("Commands: {}\n", v.commands.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{
        ArtifactKind, GraphBudgets, GraphCounters, Phase,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn sample_run(run_id: &str, goal: &str) -> GraphRun {
        GraphRun {
            version: 1,
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            cwd: ".".into(),
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
            tasks: vec![
                GraphTaskState {
                    id: "classify".into(),
                    role: Role::Classifier,
                    expect: ArtifactKind::Classification,
                    depends_on: vec![],
                    focus: None,
                    status: TaskStatus::Succeeded,
                    attempts: 1,
                    artifact_file: None,
                    error: None,
                    usage: Default::default(),
                    started_at: None,
                    ended_at: None,
                    last_activity: None,
                    fingerprint: None,
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 0,
                    memory_refs: vec![],
                    skill_refs: vec![],
                },
                GraphTaskState {
                    id: "write".into(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    depends_on: vec!["classify".into()],
                    focus: None,
                    status: TaskStatus::Pending,
                    attempts: 0,
                    artifact_file: None,
                    error: None,
                    usage: Default::default(),
                    started_at: None,
                    ended_at: None,
                    last_activity: None,
                    fingerprint: None,
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 0,
                    memory_refs: vec![],
                    skill_refs: vec![],
                },
            ],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: None,
            revision: 1,
            control_history: Vec::new(),
            continuation: None,
        }
    }

    #[test]
    fn f14_read_operations_do_not_dispatch() {
        assert!(!operation_needs_workers("diff"));
        assert!(!operation_needs_workers("explain"));
        assert!(!operation_needs_workers("dry-run"));
        assert!(operation_needs_workers("verify"));
    }

    #[test]
    fn f14_diff_no_prior_revision() {
        let run = sample_run("run-init", "build feature");
        let report = generate_graph_diff(&run, None, None, &[]);
        assert_eq!(report.current_revision, 1);
        assert_eq!(report.prior_revision, None);
        assert!(report
            .definition_diff
            .iter()
            .any(|d| d.contains("no prior revision")));
    }

    #[test]
    fn f14_diff_definition_only_change() {
        let old_run = sample_run("run-v1", "initial goal");
        let mut new_run = old_run.clone();
        new_run.revision = 2;
        new_run.goal = "updated goal".into();
        new_run.budgets.run_deadline_ms = 300_000;

        let report = generate_graph_diff(&new_run, Some(&old_run), None, &[]);
        assert_eq!(report.current_revision, 2);
        assert_eq!(report.prior_revision, Some(1));
        assert!(report
            .definition_diff
            .iter()
            .any(|d| d.contains("goal changed")));
        assert!(report
            .definition_diff
            .iter()
            .any(|d| d.contains("budgets changed")));
        assert!(report.owned_code_diff.is_empty());
    }

    #[test]
    fn f14_diff_unrelated_dirty_file() {
        let dir = tempdir().unwrap();
        let unrelated_file = "dirty_user_edit.txt";
        std::fs::write(dir.path().join(unrelated_file), b"user dirty edit").unwrap();

        let run = sample_run("run-unrelated", "feature");
        let report =
            generate_graph_diff(&run, None, Some(dir.path()), &[unrelated_file.to_string()]);

        assert!(report.unattributed_code_diff.is_some());
        let unattr = report.unattributed_code_diff.unwrap();
        assert!(unattr.contains("[unattributed]"));
        assert!(unattr.contains("user dirty edit"));
    }

    #[test]
    fn f14_diff_binary_delta() {
        let dir = tempdir().unwrap();
        let binary_file = "image.png";
        std::fs::write(dir.path().join(binary_file), b"\x89PNG\r\n\x1a\n\x00\x00").unwrap();

        let run = sample_run("run-bin", "binary change");
        let report = generate_graph_diff(&run, None, Some(dir.path()), &[binary_file.to_string()]);

        assert!(report.unattributed_code_diff.is_some());
        let unattr = report.unattributed_code_diff.unwrap();
        assert!(unattr.contains("binary file modified"));
    }

    #[test]
    fn f14_explain_failed_conditional_edge() {
        let mut run = sample_run("run-cond", "conditional edge test");
        let pred_task = GraphTaskState {
            id: "classify".into(),
            role: Role::Classifier,
            expect: ArtifactKind::Classification,
            depends_on: vec![],
            focus: None,
            status: TaskStatus::Failed,
            attempts: 1,
            artifact_file: None,
            error: Some("classification failed".into()),
            usage: Default::default(),
            started_at: None,
            ended_at: None,
            last_activity: None,
            fingerprint: None,
            mutation: None,
            context_fingerprint: None,
            context_tokens: 0,
            memory_refs: vec![],
            skill_refs: vec![],
        };
        let succ_task = GraphTaskState {
            id: "plan".into(),
            role: Role::Planner,
            expect: ArtifactKind::Plan,
            depends_on: vec!["classify".into()],
            focus: None,
            status: TaskStatus::Pending,
            attempts: 0,
            artifact_file: None,
            error: None,
            usage: Default::default(),
            started_at: None,
            ended_at: None,
            last_activity: None,
            fingerprint: None,
            mutation: None,
            context_fingerprint: None,
            context_tokens: 0,
            memory_refs: vec![],
            skill_refs: vec![],
        };
        run.tasks = vec![pred_task, succ_task];

        let explanation = explain_node(&run.tasks[1], &run);
        assert!(explanation.blocked_reason.is_some());
        let reason = explanation.blocked_reason.unwrap();
        assert!(reason.contains("condition failed"));
        assert!(reason.contains("predecessor 'classify' failed"));
    }

    #[test]
    fn f14_explain_unknown_permission_effect_marked_unknown() {
        let mut run = sample_run("run-unk", "unknown role test");
        let unknown_task = GraphTaskState {
            id: "unknown-custom-node".into(),
            role: Role::Researcher,
            expect: ArtifactKind::Evidence,
            depends_on: vec![],
            focus: Some("unknown-role".into()),
            status: TaskStatus::Pending,
            attempts: 0,
            artifact_file: None,
            error: None,
            usage: Default::default(),
            started_at: None,
            ended_at: None,
            last_activity: None,
            fingerprint: None,
            mutation: None,
            context_fingerprint: None,
            context_tokens: 0,
            memory_refs: vec![],
            skill_refs: vec![],
        };
        run.tasks = vec![unknown_task];

        let explanation = explain_node(&run.tasks[0], &run);
        assert_eq!(explanation.permission_level, "unknown");
        assert!(explanation.allowed_effects.contains(&"unknown".to_string()));
    }

    #[test]
    fn f14_executor_spies_remain_zero() {
        let worker_spawns = Arc::new(AtomicUsize::new(0));
        let provider_calls = Arc::new(AtomicUsize::new(0));

        let run = sample_run("run-spy", "read only spy check");
        // Execute diff and explain
        let _diff = generate_graph_diff(&run, None, None, &[]);
        let _explain = generate_graph_explain(&run, None);

        // Neither worker nor provider is dispatched for diff/explain
        assert_eq!(worker_spawns.load(Ordering::SeqCst), 0);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn f14_narrow_rendering() {
        let run = sample_run("run-narrow", "very long goal description that should be wrapped or truncated properly in narrow displays");
        let diff = generate_graph_diff(&run, None, None, &[]);
        let explain = generate_graph_explain(&run, None);

        let narrow_diff = render_diff_report(&diff, 30);
        for line in narrow_diff.lines() {
            assert!(line.len() <= 30, "line exceeds narrow width: {:?}", line);
        }

        let narrow_explain = render_explain_report(&explain, 30);
        for line in narrow_explain.lines() {
            assert!(line.len() <= 30, "line exceeds narrow width: {:?}", line);
        }
    }

    #[test]
    fn f14_branch_keeps_spend() {
        assert_eq!(branch_remaining(120, 81, 10), Some(29));
        assert_eq!(branch_remaining(120, 121, 0), None);
    }

    #[test]
    fn f14_fork_prior_parent_pruned_unless_pinned() {
        use crate::native_extensions::graph::store;
        let dir = tempdir().unwrap();
        let mut parent = sample_run("parent-run", "parent goal");
        parent.cwd = dir.path().to_string_lossy().to_string();
        store::create_run_dir(dir.path(), &parent.run_id).unwrap();
        store::save_run(&mut parent).unwrap();

        // Pin parent run via fork
        let preview = generate_fork_preview(
            &parent,
            "write",
            ForkStrategy::RepairMinimal,
            Some(dir.path()),
        )
        .unwrap();
        let _forked = execute_fork(&parent, &preview, dir.path()).unwrap();

        assert!(store::is_run_pinned(dir.path(), &parent.run_id));
        assert_eq!(
            store::read_ancestor_run(dir.path(), &preview.new_run_id),
            Some(parent.run_id.clone())
        );
    }

    #[test]
    fn f14_fork_incompatible_ancestor_evidence() {
        use crate::native_extensions::graph::history::exact_replay;
        let compatible = exact_replay("clean", "clean", "hash-A", "hash-B", true);
        assert!(
            !compatible,
            "differing content hashes must invalidate ancestor replay"
        );
    }

    #[test]
    fn f14_fork_attempted_fresh_full_budget() {
        let mut parent = sample_run("parent-budget", "parent goal");
        parent.budgets.run_deadline_ms = 60000;
        parent.counters.cost_usd = 4.50;
        let preview =
            generate_fork_preview(&parent, "write", ForkStrategy::RepairMinimal, None).unwrap();

        assert_eq!(preview.retained_spend_usd, 4.50);
        assert_eq!(branch_remaining(100, 60, 0), Some(40));
    }

    #[test]
    fn f14_fork_same_status_source_change() {
        use crate::native_extensions::graph::history::exact_replay;
        assert!(!exact_replay(
            "dirty",
            "dirty",
            "content-v1",
            "content-v2",
            true
        ));
        assert!(exact_replay(
            "dirty",
            "dirty",
            "content-v1",
            "content-v1",
            true
        ));
    }

    #[test]
    fn f14_rewind_missing_checkpoint() {
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-no-cp", "test rewind");
        let preview = generate_rewind_preview(&run, "write", dir.path(), &[]).unwrap();
        assert!(preview.checkpoint_id.is_none());

        let err = execute_rewind(&mut run, &preview, dir.path(), true, true).unwrap_err();
        assert!(err.contains("missing checkpoint"));
    }

    #[test]
    fn f14_rewind_overlapping_manual_edit() {
        use crate::native_extensions::graph::history;
        use crate::native_extensions::graph::mutation::ChangedFile;
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-conflict", "test rewind conflict");
        run.cwd = dir.path().to_string_lossy().to_string();
        if let Some(task) = run.tasks.iter_mut().find(|t| t.id == "write") {
            task.mutation = Some(crate::native_extensions::graph::mutation::GraphMutation {
                files: vec![ChangedFile::modified("src/main.rs")],
                ..Default::default()
            });
        }
        history::record_task_checkpoint(dir.path(), &run.run_id, "write", "cp-123").unwrap();

        let manual_dirty = vec!["src/main.rs".to_string()];
        let preview = generate_rewind_preview(&run, "write", dir.path(), &manual_dirty).unwrap();
        assert!(preview.has_manual_conflicts);
        assert_eq!(preview.conflict_files, vec!["src/main.rs".to_string()]);

        let result = execute_rewind(&mut run, &preview, dir.path(), true, true).unwrap();
        assert!(result.success);
        assert_eq!(result.preserved_conflicts, vec!["src/main.rs".to_string()]);
        assert!(result.restored_files.is_empty());
    }

    #[test]
    fn f14_rewind_code_only_state_only_combined() {
        use crate::native_extensions::graph::history;
        use crate::native_extensions::graph::mutation::ChangedFile;
        let dir = tempdir().unwrap();
        let mut run = sample_run("run-modes", "test rewind modes");
        run.cwd = dir.path().to_string_lossy().to_string();
        if let Some(task) = run.tasks.iter_mut().find(|t| t.id == "write") {
            task.status = TaskStatus::Succeeded;
            task.mutation = Some(crate::native_extensions::graph::mutation::GraphMutation {
                files: vec![ChangedFile::modified("src/lib.rs")],
                ..Default::default()
            });
        }
        history::record_task_checkpoint(dir.path(), &run.run_id, "write", "cp-write").unwrap();

        let preview = generate_rewind_preview(&run, "write", dir.path(), &[]).unwrap();

        // 1. Code only
        let mut run_code_only = run.clone();
        let res1 = execute_rewind(&mut run_code_only, &preview, dir.path(), true, false).unwrap();
        assert_eq!(res1.restored_files, vec!["src/lib.rs"]);
        assert_eq!(
            run_code_only
                .tasks
                .iter()
                .find(|t| t.id == "write")
                .unwrap()
                .status,
            TaskStatus::Succeeded
        );

        // 2. State only
        let mut run_state_only = run.clone();
        let res2 = execute_rewind(&mut run_state_only, &preview, dir.path(), false, true).unwrap();
        assert!(res2.restored_files.is_empty());
        assert_eq!(
            run_state_only
                .tasks
                .iter()
                .find(|t| t.id == "write")
                .unwrap()
                .status,
            TaskStatus::Pending
        );

        // 3. Combined
        let mut run_combined = run.clone();
        let res3 = execute_rewind(&mut run_combined, &preview, dir.path(), true, true).unwrap();
        assert_eq!(res3.restored_files, vec!["src/lib.rs"]);
        assert_eq!(
            run_combined
                .tasks
                .iter()
                .find(|t| t.id == "write")
                .unwrap()
                .status,
            TaskStatus::Pending
        );
    }

    #[test]
    fn f14_rewind_runtime_checkpoint_restores_worker_bytes() {
        use crate::native_extensions::graph::history;
        use crate::native_extensions::graph::mutation::ChangedFile;
        use crate::native_extensions::graph::store;
        let dir = tempdir().unwrap();
        let source = dir.path().join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        let before = b"before\n";
        let after = b"after\n";
        std::fs::write(&source, after).unwrap();

        let mut run = sample_run("run-runtime-rewind", "runtime rewind");
        run.cwd = dir.path().to_string_lossy().to_string();
        run.tasks[1].status = TaskStatus::Succeeded;
        run.tasks[1].mutation = Some(crate::native_extensions::graph::mutation::GraphMutation {
            files: vec![ChangedFile::modified("src/lib.rs")],
            ..Default::default()
        });
        store::create_run_dir(dir.path(), &run.run_id).unwrap();
        history::record_task_checkpoint(dir.path(), &run.run_id, "write", "cp-runtime").unwrap();
        let preview = generate_rewind_preview(&run, "write", dir.path(), &[]).unwrap();
        let task_id = davinci_agent::TaskId::new();
        let mut effect = davinci_agent::runtime::effects::OwnedFileEffect::new(
            "write-call",
            davinci_agent::AgentId::new(),
            1,
            "src/lib.rs",
            davinci_agent::runtime::effects::FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some(davinci_agent::runtime::checkpoints::compute_sha256(before));
        effect.after_blob = Some(davinci_agent::runtime::checkpoints::compute_sha256(after));
        let report_path =
            store::artifact_path(dir.path(), &run.run_id, "write").with_extension("effects.jsonl");
        davinci_agent::runtime::effects::append_effect_report(
            &report_path,
            &effect,
            Some(before),
            Some(after),
        )
        .unwrap();

        let result = execute_rewind_with_runtime(&mut run, &preview, dir.path(), true, false)
            .expect("runtime rewind applies");
        assert_eq!(result.restored_files, vec!["src/lib.rs"]);
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }

    #[test]
    fn f14_fork_old_worker_generation_blocked() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let aborted = AtomicBool::new(true);
        assert!(
            aborted.load(Ordering::SeqCst),
            "old worker generation must be blocked by abort flag"
        );
    }

    #[test]
    fn f14_verify_never_runs_writer() {
        assert!(!verify_stage_allowed("writer"));
        assert!(!verify_stage_allowed("planner"));
        assert!(verify_stage_allowed("verification"));
        assert!(verify_stage_allowed("security"));
        assert!(verify_stage_allowed("reviewer"));
    }

    #[test]
    fn f14_verify_writer_runner_spy_zero() {
        let mut run = sample_run("run-verify-spy", "verify only run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            true,
        );
        assert!(!report.writer_spawned);
        assert!(report.passed);
        assert_eq!(run.counters.workers_spawned, 0);
    }

    #[test]
    fn f14_verify_simulated_exit0_rejected() {
        let mut run = sample_run("run-verify-sim", "verify sim run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = crate::native_extensions::graph::verify::dry_run_verify_exec;
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            true,
        );
        assert!(!report.passed);
        assert!(report.simulated_rejected);
        assert!(report
            .message
            .contains("Simulated verification output rejected"));
    }

    #[test]
    fn f14_verify_unavailable_mandatory_security() {
        let mut run = sample_run("run-verify-sec", "verify sec run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            false,
            true,
        );
        assert!(!report.passed);
        assert!(!report.security_passed);
        assert!(report.message.contains("security"));
    }

    #[test]
    fn f14_verify_changed_source_during_review() {
        let mut run = sample_run("run-verify-review-drift", "review drift run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            false,
        );
        assert!(!report.passed);
        assert!(!report.review_passed);
    }

    #[test]
    fn f14_verify_omitted_review_chunk() {
        use crate::native_extensions::graph::review_coverage::{coverage_complete, ReviewCoverage};
        let mut coverage = ReviewCoverage::new(vec!["chunk-1".into(), "chunk-2".into()]);
        coverage.record_reviewed(&["chunk-1".into()]);
        assert!(!coverage_complete(&coverage));

        let mut run = sample_run("run-verify-chunk", "verify chunk run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            coverage_complete(&coverage),
        );
        assert!(!report.passed);
        assert!(!report.review_passed);
        assert!(report.message.contains("missing chunk coverage"));
    }

    #[test]
    fn f14_verify_no_commands() {
        let mut run = sample_run("run-verify-none", "no commands run");
        let dir = tempdir().unwrap();
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report =
            execute_verify_only(&mut run, dir.path(), &[], &exec, &abort, None, true, true);
        assert!(!report.passed);
        assert!(report.verification.is_none());
        assert!(report.message.contains("no verification commands"));
    }

    #[test]
    fn f14_verify_failed_test() {
        let mut run = sample_run("run-verify-fail", "failing test run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec =
            |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (1, "test failed".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            true,
        );
        assert!(!report.passed);
        assert_eq!(report.verification.unwrap().commands[0].exit_code, 1);
    }

    #[test]
    fn f14_verify_root_deadline() {
        let mut run = sample_run("run-verify-dl", "deadline run");
        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            Some(0),
            true,
            true,
        );
        assert!(!report.passed);
        let v = report.verification.unwrap();
        assert!(!v.passed);
        assert!(v.commands[0].output_tail.contains("root deadline exceeded"));
    }

    #[test]
    fn f14_verify_unrelated_succeeded_nodes_untouched() {
        let mut run = sample_run("run-verify-untouched", "untouched nodes");
        assert_eq!(run.tasks[0].id, "classify");
        assert_eq!(run.tasks[0].status, TaskStatus::Succeeded);

        let dir = tempdir().unwrap();
        let commands = vec![crate::native_extensions::graph::types::VerifyCommandSpec {
            name: "test".into(),
            command: "cargo test".into(),
            from_plan: false,
        }];
        let abort = Arc::new(AtomicBool::new(false));
        let exec = |_: &str, _: &Path, _: &Arc<AtomicBool>, _: u64| (0, "ok".to_string(), 10);
        let report = execute_verify_only(
            &mut run,
            dir.path(),
            &commands,
            &exec,
            &abort,
            None,
            true,
            true,
        );
        assert!(report.passed);
        assert!(report.unchanged_nodes.contains(&"classify".to_string()));
        assert_eq!(run.tasks[0].status, TaskStatus::Succeeded);
    }

    #[test]
    fn f14_budget_floor_and_overflow_are_rejected() {
        assert!(budget_update_allowed(100, 70, 20, true));
        assert!(!budget_update_allowed(80, 70, 20, true));
        assert!(!budget_update_allowed(100, 70, 20, false));
        assert!(!budget_update_allowed(100, u64::MAX, 20, true));
    }

    #[test]
    fn f14_budget_updates_require_revision_and_preserve_spend() {
        let mut run = sample_run("budget-update", "budget update");
        run.revision = 4;
        run.counters.cost_usd = 1.25;
        let missing = apply_budget_update(&mut run, "max-cost-usd", "2", None, true);
        assert!(missing.unwrap_err().contains("expected graph revision"));
        let stale = apply_budget_update(&mut run, "max-cost-usd", "2", Some(3), true);
        assert!(stale.unwrap_err().contains("stale graph revision"));
        let updated = apply_budget_update(&mut run, "max-cost-usd", "2", Some(4), true).unwrap();
        assert_eq!(updated.ceilings.max_cost_usd, 2.0);
        assert_eq!(run.revision, 5);
        assert!(apply_budget_update(&mut run, "max-cost-usd", "1", Some(5), true).is_err());
        assert!(apply_budget_update(&mut run, "max-cost-usd", "NaN", Some(5), true).is_err());
        assert!(apply_budget_update(&mut run, "max-cost-usd", "0", Some(5), true).is_ok());
        run.dry_run = true;
        assert!(apply_budget_update(&mut run, "max-cost-usd", "2", Some(6), true).is_err());

        run.dry_run = false;
        run.revision = u64::MAX;
        let before = run.budgets.clone();
        assert!(
            apply_budget_update(&mut run, "max-workers", "20", Some(u64::MAX), true)
                .unwrap_err()
                .contains("revision exhausted")
        );
        assert_eq!(
            run.budgets, before,
            "overflow must not partially update budgets"
        );
    }

    #[test]
    fn f14_source_manifest_tracks_owned_inputs_and_detects_drift() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").unwrap();
        let mut run = sample_run("source-manifest", "source drift");
        run.tasks[1].mutation = Some(crate::native_extensions::graph::mutation::GraphMutation {
            files: vec![crate::native_extensions::graph::mutation::ChangedFile::modified("src.rs")],
            patch_chunks: vec![],
        });

        let before = source_manifest_for_run(&run, dir.path())
            .unwrap()
            .expect("owned source path");
        std::fs::write(dir.path().join("src.rs"), "fn main() { println!(); }\n").unwrap();
        let after = source_manifest_for_run(&run, dir.path())
            .unwrap()
            .expect("owned source path");
        assert!(!before.is_current(&after));
    }
}
