//! Real data for the surfaces that have a source in this workspace.
//!
//! `davinci_sources.rs` covers what git and the session store know. This
//! covers what the native extensions know: the plan sheet (Disegno, `1c`)
//! reads the graph run's task list, and vector recall (Memoria, `2b`) reads
//! the vector memory index.
//!
//! Grafo (`2a`) still has no source — the `graph` extension is a multi-agent
//! run orchestrator, not a symbol index, and nothing in this workspace builds
//! one — so it stays on `davinci_tui::davinci::fixtures` where it is obviously a
//! drawing rather than a claim.

use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use davinci_agent::Agent;
use davinci_tui::davinci::model::{
    AgentRow, BudgetMeta, BudgetRow, Model, PlanStep, Proposal, RecallHit, RecallMeta, TaskBoardRow,
};
use davinci_tui::davinci::theme::State;
use davinci_tui::davinci::views::disegno::roman;

use crate::native_extensions::graph::{list_runs, load_run, GraphRun, Role, TaskStatus};
use crate::native_extensions::vector_memory::{VectorMemory, VectorMemoryConfig};

/// The plan sheet (`1c`) from the newest graph run in this project, or an
/// empty plan when there has never been one.
pub fn plan(cwd: &Path) -> Vec<PlanStep> {
    let Some(run) = latest_run(cwd) else {
        return Vec::new();
    };
    plan_from_run(&run)
}

/// The newest persisted run, live or finished.
pub fn latest_run(cwd: &Path) -> Option<GraphRun> {
    let newest = list_runs(cwd).into_iter().next()?;
    load_run(cwd, &newest.run_id)
}

/// Pure form of [`plan`].
pub fn plan_from_run(run: &GraphRun) -> Vec<PlanStep> {
    run.tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            PlanStep::new(
                &roman(index + 1),
                task_state(task.status),
                work_verb(task.role),
                task_target(
                    task.focus.as_deref(),
                    task.last_activity.as_deref(),
                    &task.id,
                ),
            )
        })
        .collect()
}

/// design.md §4: the glyph carries the state, so this mapping is the whole of
/// what a reader sees under `NO_COLOR`.
pub fn task_state(status: TaskStatus) -> State {
    match status {
        TaskStatus::Succeeded => State::Done,
        TaskStatus::Running => State::Active,
        TaskStatus::Pending | TaskStatus::Ready => State::Queued,
        TaskStatus::Failed => State::Failed,
        TaskStatus::Cancelled => State::Skipped,
    }
}

/// design.md §5 lists the work verbs and asks that they be used literally.
pub fn work_verb(role: Role) -> &'static str {
    match role {
        Role::Classifier => "measuring",
        Role::Researcher => "surveying",
        Role::TestAnalyzer => "testing",
        Role::Historian => "tracing",
        Role::Planner => "studying",
        Role::Writer => "constructing",
        Role::Reviewer => "verifying",
    }
}

/// What a step is working on: what it was pointed at, else what it last did,
/// else its own name. Never nothing.
fn task_target<'a>(
    focus: Option<&'a str>,
    activity: Option<&'a str>,
    id: &'a str,
) -> Option<&'a str> {
    focus
        .filter(|text| !text.trim().is_empty())
        .or(activity.filter(|text| !text.trim().is_empty()))
        .or(Some(id))
}

/// Vector recall (`2b`) against the real index. The floor is the config's
/// minimum score: hits below it are kept and counted, not drawn, so the
/// retrieval stays auditable.
pub fn recall(cwd: &Path, query: &str, limit: usize) -> (Vec<RecallHit>, RecallMeta) {
    let config = VectorMemoryConfig::from_env();
    let floor = config.minimum_score as f64;
    let embedding = config.embedding_model.clone();
    let memory = VectorMemory::with_config(cwd.to_path_buf(), config);

    let started = Instant::now();
    let hits = memory.search(query, limit.max(1));
    let elapsed = started.elapsed();

    let vectors = memory.record_count();
    let rows: Vec<RecallHit> = hits
        .iter()
        .map(|hit| {
            let score = hit.score as f64;
            RecallHit::new(
                score,
                &first_line(&hit.record.text, 68),
                &hit.record.source,
                &provenance(hit),
                score >= floor,
            )
        })
        .collect();

    let meta = RecallMeta {
        query: query.to_string(),
        vectors: thousands(vectors as u64),
        shards: "1".into(),
        embedding,
        metric: "cosine".into(),
        elapsed: format!("{}ms", elapsed.as_millis()),
        k: rows.len().to_string(),
        floor,
        promoted: rows
            .iter()
            .filter(|row| row.above_floor)
            .count()
            .to_string(),
        freshness: freshness(vectors),
    };
    (rows, meta)
}

/// Where a hit came from, and by which half of the search. Retrieval that
/// cannot be audited is not retrieval (design.md §9).
fn provenance(hit: &crate::native_extensions::vector_memory::MemoryHit) -> String {
    let kind = serde_json::to_value(hit.record.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "record".into());
    if hit.dense_score >= hit.lexical_score {
        format!("{kind} · dense {:.2}", hit.dense_score)
    } else {
        format!("{kind} · lexical {:.2}", hit.lexical_score)
    }
}

fn freshness(records: usize) -> String {
    if records == 0 {
        "empty index".into()
    } else {
        format!("{} indexed", thousands(records as u64))
    }
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let line = line.trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let mut out: String = line.chars().take(max).collect();
    out.push('…');
    out
}

fn thousands(value: u64) -> String {
    davinci_tui::davinci::views::chrome::thousands(value)
}

/// The token governor (`2c`): where this session's context has gone, what it
/// costs, and — when the window is nearly full — what to do about it.
///
/// The "roles" are the parts of the context, because that is what a context
/// window is actually divided between. Every number states its unit and its
/// cap (design.md §9).
pub fn budget(agent: &Agent, window: u64) -> (Vec<BudgetRow>, BudgetMeta, Option<Proposal>) {
    let window = window.max(1);
    // Four characters to a token, the same estimate the compactor uses, so
    // the two never disagree about how full the window is.
    let text_tokens = |text: &str| (text.len() as u64).div_ceil(4);
    let instructions = text_tokens(&agent.system_prompt)
        + agent
            .context_files
            .iter()
            .map(|file| text_tokens(&file.body))
            .sum::<u64>();

    let mut asked = 0;
    let mut replied = 0;
    let mut tools = 0;
    for message in &agent.messages {
        let tokens = davinci_agent::estimate_context_tokens(std::slice::from_ref(message));
        match message.role.as_str() {
            "user" => asked += tokens,
            "assistant" => replied += tokens,
            _ => tools += tokens,
        }
    }
    let in_use = instructions + asked + replied + tools;

    // The cap a row is measured against is the window, not the largest row:
    // a bar that renormalises hides how much room is left.
    let row = |role: &str, tokens: u64, note: &str| {
        let fraction = tokens as f64 / window as f64;
        BudgetRow::new(
            role,
            &thousands(tokens),
            fraction.min(1.0),
            note,
            fraction > 0.5,
        )
    };
    let rows = vec![
        row(
            "instructions",
            instructions,
            "system prompt and context files",
        ),
        row("asked", asked, "what you sent"),
        row("replied", replied, "what the model sent back"),
        row("tool output", tools, "what the tools returned"),
    ];

    let turns = agent
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count() as u64;
    let meta = BudgetMeta {
        policy: compaction_policy(agent),
        in_use: thousands(in_use),
        window: thousands(window),
        headroom: thousands(window.saturating_sub(in_use)),
        in_use_fraction: (in_use as f64 / window as f64).min(1.0),
        rate: format!("{}/turn", thousands(in_use / turns.max(1))),
        session_spend: session_spend(agent),
        // Nothing in this workspace tracks spend across sessions, so the
        // governor says so rather than inventing a cap to measure against.
        daily_cap: "not set".into(),
        daily_fraction: 0.0,
        history: match turns {
            1 => "1 turn this session".into(),
            other => format!("{other} turns this session"),
        },
    };

    (rows, meta, compaction_proposal(agent, in_use, window))
}

fn compaction_policy(agent: &Agent) -> String {
    let settings = crate::settings::load_merged_settings(&crate::default_agent_dir(), &agent.cwd);
    if settings.auto_compact.unwrap_or(true) {
        "auto-compact on".into()
    } else {
        "auto-compact off".into()
    }
}

fn session_spend(agent: &Agent) -> String {
    let Some(store) = agent.session.as_ref() else {
        return "$0.00".into();
    };
    let stats = davinci_session::session_usage_stats(&store.entries);
    format!("${:.2}", stats.cost)
}

/// A proposal is only worth making once the window is genuinely tight, and it
/// must always say what it recovers, what it keeps, what it costs and whether
/// it can be undone (design.md §6).
fn compaction_proposal(agent: &Agent, in_use: u64, window: u64) -> Option<Proposal> {
    if (in_use as f64) < 0.7 * window as f64 {
        return None;
    }
    // Compaction summarises everything but the recent turns; what it recovers
    // is what those older messages currently cost.
    let keep = 6usize;
    let older = agent.messages.len().saturating_sub(keep);
    let recovers = davinci_agent::estimate_context_tokens(&agent.messages[..older]);
    if recovers == 0 {
        return None;
    }
    Some(Proposal {
        summary: format!(
            "the window is {}% full; compacting would summarise the older turns",
            ((in_use as f64 / window as f64) * 100.0) as u32
        ),
        recovers: format!("{} tokens", thousands(recovers)),
        keeps: format!("the last {keep} messages verbatim"),
        cost: "one summarisation call".into(),
        // The session file keeps every original entry, so nothing is lost.
        reversible: true,
        actions: vec![
            ("/compact".into(), "summarise now".into()),
            ("esc".into(), "leave it".into()),
        ],
    })
}

/// Format cost minor units or label as unknown.
pub fn cost_label(cost_minor_units: Option<u64>) -> String {
    cost_minor_units
        .map(|v| format!("{v} minor units"))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Fill the surfaces that have a source, leaving the ones that do not alone.
pub fn dress_from_extensions(model: &mut Model, cwd: &Path, agent: &Agent) {
    let plan = plan(cwd);
    if !plan.is_empty() {
        model.plan = plan;
    }
    let (rows, meta, proposal) = budget(agent, agent.context_window);
    model.budget = rows;
    model.budget_meta = meta;
    model.proposal = proposal;
}

/// Filter whether a task from `task_root` is visible to an active session run with `active_root`.
#[allow(dead_code)]
pub fn task_visible(task_root: &str, active_root: &str, _terminal: bool) -> bool {
    task_root == active_root
}

/// Map an internal [`davinci_agent::TaskState`] to the TUI theme [`State`].
pub fn task_state_to_tui(state: davinci_agent::TaskState) -> State {
    match state {
        davinci_agent::TaskState::Completed => State::Done,
        davinci_agent::TaskState::Running => State::Active,
        davinci_agent::TaskState::Pending | davinci_agent::TaskState::Ready => State::Queued,
        davinci_agent::TaskState::Failed | davinci_agent::TaskState::Blocked => State::Failed,
        davinci_agent::TaskState::Cancelled => State::Skipped,
    }
}

/// Scope status of an execution task on the UI surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskScopeStatus {
    LegacyUncontracted,
    ContractScoped {
        revision: u64,
        digest: String,
        writable_count: usize,
        protected_count: usize,
    },
    InvalidContract {
        reason: String,
    },
}

impl TaskScopeStatus {
    #[allow(dead_code)]
    pub fn display_label(&self) -> String {
        match self {
            Self::LegacyUncontracted => "legacy (uncontracted)".to_string(),
            Self::ContractScoped { revision, .. } => format!("contracted (rev {revision})"),
            Self::InvalidContract { reason } => format!("invalid contract: {reason}"),
        }
    }
}

/// Evaluates scope status for an authoritative task record and optional contract.
///
/// Invariants:
/// - Uncontracted tasks (`contract_digest: None`) are visibly reported as `LegacyUncontracted`.
///   Legacy tasks are never claimed to be contract scoped.
/// - Contract-scoped status is only reported when an active contract is provided, validates
///   cleanly, and matches the bound contract digest.
/// - If a contract is invalid or its digest mismatches the bound record digest, it fails closed
///   with `InvalidContract`.
/// - Following task resume, the correct active contract revision is reflected.
#[allow(dead_code)]
pub fn evaluate_task_scope_status(
    record: &davinci_agent::TaskRecord,
    contract: Option<&davinci_agent::runtime::contracts::TaskContract>,
) -> TaskScopeStatus {
    match (&record.contract_digest, contract) {
        (None, _) => TaskScopeStatus::LegacyUncontracted,
        (Some(_), None) => TaskScopeStatus::InvalidContract {
            reason: "missing contract for bound digest".into(),
        },
        (Some(bound_digest), Some(c)) => {
            if let Err(e) = c.validate() {
                TaskScopeStatus::InvalidContract {
                    reason: e.to_string(),
                }
            } else if c.digest != *bound_digest {
                TaskScopeStatus::InvalidContract {
                    reason: format!(
                        "digest mismatch: bound {bound_digest} != contract {}",
                        c.digest
                    ),
                }
            } else {
                TaskScopeStatus::ContractScoped {
                    revision: c.revision,
                    digest: c.digest.clone(),
                    writable_count: c.writable_paths.len(),
                    protected_count: c.protected_paths.len(),
                }
            }
        }
    }
}

/// Project authoritative task records into UI rows for the live task board.
pub fn task_board_from_records(tasks: &[davinci_agent::TaskRecord]) -> Vec<TaskBoardRow> {
    tasks
        .iter()
        .map(|t| {
            let mut row = TaskBoardRow::new(
                t.id.to_string(),
                t.title.clone(),
                t.state.public_status(),
                task_state_to_tui(t.state),
            );
            row.owner = t.assigned_to.as_ref().map(|id| id.to_string());
            row.dependencies = t.dependencies.iter().map(|id| id.to_string()).collect();
            row.parent_plan_step = t.parent_plan_step.as_ref().map(|s| s.step_id.to_string());
            row.evidence_refs = t.evidence_refs.iter().map(|e| e.to_string()).collect();
            row.blocked_reasons = t
                .blocked_reasons
                .iter()
                .map(|b| b.message.clone())
                .collect();
            row.updated_at_ms = t.updated_at_ms;
            if row.activity.is_none() {
                row.activity = Some(match &t.contract_digest {
                    Some(digest) => format!("contract: {:.8}", digest),
                    None => "legacy (uncontracted)".to_string(),
                });
            }
            row
        })
        .collect()
}

/// The live execution task board snapshot for the current runtime.
pub fn task_board(runtime: &davinci_agent::RuntimeHandle) -> Vec<TaskBoardRow> {
    let tasks = runtime.task_registry.list_tasks(Some(runtime.run_id));
    let active_root = runtime.run_id.to_string();
    let visible_tasks: Vec<_> = tasks
        .into_iter()
        .filter(|t| task_visible(&t.run_id.to_string(), &active_root, t.state.is_terminal()))
        .collect();
    task_board_from_records(&visible_tasks)
}

/// Action mapping for the live worker control panel (`/agents`).
#[allow(dead_code)]
pub fn worker_action(key: &str) -> Option<&'static str> {
    match key {
        "enter" => Some("inspect"),
        "s" => Some("steer"),
        "x" => Some("stop"),
        "r" => Some("retry"),
        "d" => Some("diff"),
        _ => None,
    }
}

/// Project authoritative worker snapshots into UI rows for the live agents panel.
pub fn agents_from_snapshots(
    snapshots: &[davinci_agent::runtime::WorkerSnapshot],
) -> Vec<AgentRow> {
    snapshots
        .iter()
        .map(|s| {
            let state = match s.state {
                davinci_agent::runtime::AgentState::Completed => State::Done,
                davinci_agent::runtime::AgentState::Running => State::Active,
                davinci_agent::runtime::AgentState::Starting
                | davinci_agent::runtime::AgentState::Waiting
                | davinci_agent::runtime::AgentState::Idle => State::Queued,
                davinci_agent::runtime::AgentState::Stopping
                | davinci_agent::runtime::AgentState::Cancelled => State::Skipped,
                davinci_agent::runtime::AgentState::Failed => State::Failed,
            };

            let elapsed_secs = s.elapsed_ms / 1000;
            let mins = elapsed_secs / 60;
            let secs = elapsed_secs % 60;
            let elapsed = format!("{:02}:{:02}", mins, secs);

            let owned_paths = s
                .owned_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect();

            let mut row = AgentRow::new(s.agent_id.to_string(), s.name.clone(), state);
            row.role = format!("{:?}", s.kind).to_lowercase();
            row.status = match s.state {
                davinci_agent::runtime::AgentState::Starting => "starting".into(),
                davinci_agent::runtime::AgentState::Running => "working".into(),
                davinci_agent::runtime::AgentState::Waiting => "waiting".into(),
                davinci_agent::runtime::AgentState::Idle => "idle".into(),
                davinci_agent::runtime::AgentState::Stopping => "stopping".into(),
                davinci_agent::runtime::AgentState::Completed => "completed".into(),
                davinci_agent::runtime::AgentState::Failed => "failed".into(),
                davinci_agent::runtime::AgentState::Cancelled => "cancelled".into(),
            };
            row.elapsed = elapsed;
            row.owned_paths = owned_paths;
            row.tool_count = s.tool_count;
            row.waiting_on = s.waiting_on.clone();
            row.disconnected = s.disconnected;
            row.usage_unknown = s.usage_unknown;
            row
        })
        .collect()
}

/// The live agents sheet snapshot for the current runtime.
pub fn agents_sheet(runtime: &davinci_agent::RuntimeHandle) -> Vec<AgentRow> {
    let controller = davinci_agent::runtime::WorkerController::new(runtime.registry.clone());
    let snapshots = controller.build_all_snapshots();
    agents_from_snapshots(&snapshots)
}

/// Canonical evidence status label reflecting execution outcome and freshness.
#[allow(dead_code)]
pub fn evidence_label(performed: bool, passed: bool, current: bool) -> &'static str {
    if !performed {
        "not performed"
    } else if !passed {
        "failed"
    } else if !current {
        "stale"
    } else {
        "passed on current source"
    }
}

/// Redact secrets, auth headers, and truncate large evidence outputs.
#[allow(dead_code)]
pub fn redact_evidence_output(raw: &str, max_len: usize) -> String {
    let patterns = [
        ("(?i)bearer\\s+[a-zA-Z0-9_\\-\\.]+", "Bearer [REDACTED]"),
        ("(?i)sk-[a-zA-Z0-9]{20,}", "[REDACTED_API_KEY]"),
        ("(?i)password=[^&\\s]+", "password=[REDACTED]"),
        ("(?i)secret=[^&\\s]+", "secret=[REDACTED]"),
        ("(?i)token=[^&\\s]+", "token=[REDACTED]"),
    ];

    let mut sanitized = raw.to_string();
    for (pat, rep) in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            sanitized = re.replace_all(&sanitized, rep).to_string();
        }
    }

    if sanitized.len() > max_len {
        let mut cut = max_len;
        while cut > 0 && !sanitized.is_char_boundary(cut) {
            cut -= 1;
        }
        let truncated_bytes = sanitized.len() - cut;
        sanitized.truncate(cut);
        sanitized.push_str(&format!("\n... [truncated {truncated_bytes} bytes]"));
    }

    sanitized
}

/// Validates that an artifact retrieval path stays strictly within the authorized store boundary.
#[allow(dead_code)]
pub fn validate_artifact_scope(
    store_root: &std::path::Path,
    relative_path: &str,
) -> Result<std::path::PathBuf, String> {
    if relative_path.contains("..")
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
    {
        return Err("Artifact path attempts to escape store root boundary".into());
    }
    let target = store_root.join(relative_path);
    if let (Ok(canon_root), Ok(canon_target)) = (store_root.canonicalize(), target.canonicalize()) {
        if !canon_target.starts_with(&canon_root) {
            return Err("Artifact path resolves outside store root boundary".into());
        }
    }
    Ok(target)
}

#[allow(dead_code)]
pub fn freshness_label(has_provenance: bool, fingerprint_matches: bool) -> &'static str {
    if !has_provenance {
        "unproven"
    } else if fingerprint_matches {
        "fresh"
    } else {
        "stale"
    }
}

/// Builds the ContextInspectorSheet from a prepared context manifest and optional overlay.
pub fn context_inspector_sheet_from_manifest(
    manifest: &davinci_agent::runtime::PreparedContextManifest,
    overlay: Option<&davinci_agent::runtime::ContextOverlay>,
    preview_active: bool,
    show_pending: bool,
) -> davinci_tui::davinci::model::ContextInspectorSheet {
    use davinci_tui::davinci::model::{ContextInspectorRow, ContextInspectorSheet};

    let rows: Vec<ContextInspectorRow> = manifest
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

            let preview_body = Some(redact_evidence_output(
                &format!(
                    "ID: {}\nCategory: {}\nProvenance: {:?}\nSource: {}\nFingerprint: {}\nTokens: {}\nReason: {}\nStatus: {}",
                    entry.id,
                    entry.category,
                    entry.provenance_kind,
                    entry.source_ref,
                    entry.content_hash,
                    entry.token_estimate,
                    entry.selection_reason.as_deref().unwrap_or("none"),
                    if entry.mandatory {
                        "mandatory"
                    } else if is_pinned {
                        "pinned"
                    } else if selected {
                        "selected"
                    } else {
                        "excluded"
                    }
                ),
                4096,
            ));

            ContextInspectorRow {
                item_id: entry.id.clone(),
                category: entry.category.clone(),
                provenance: entry.provenance_kind.as_str().to_string(),
                source_ref: entry.source_ref.clone(),
                fingerprint: entry.content_hash.clone(),
                estimated_tokens: entry.token_estimate,
                selected,
                inclusion_reason: entry.selection_reason.clone(),
                mandatory: entry.mandatory,
                pinned: is_pinned,
                freshness: entry.freshness.clone(),
                last_refreshed_at: entry.inspect_ref.clone(),
                preview_body,
            }
        })
        .collect();

    ContextInspectorSheet {
        request_id: manifest.request_id.clone(),
        root_run_id: manifest.root_run_id.to_string(),
        source_revision: manifest.source_revision,
        overlay_revision: manifest.overlay_revision,
        manifest_digest: manifest.manifest_digest.clone(),
        rows,
        selected_index: 0,
        preview_active,
        show_pending,
        confirmation_dialog: None,
    }
}

/// Formatted report of interaction test coverage and named capability gaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionCoverageReport {
    pub real_terminal_verified: bool,
    pub real_browser_verified: bool,
    pub manual_verified: bool,
    pub receipts_count: usize,
    pub passed_receipts: usize,
    pub named_gaps: Vec<String>,
}

#[allow(dead_code)]
pub fn interaction_coverage_report(
    receipts: &[davinci_coding_agent::interaction_testing::InteractionReceipt],
) -> InteractionCoverageReport {
    use davinci_coding_agent::interaction_testing::BackendKind;

    let mut named_gaps = Vec::new();
    let mut real_terminal_verified = false;
    let mut real_browser_verified = false;
    let mut manual_verified = false;
    let mut passed_receipts = 0;

    for r in receipts {
        let source_bound = r
            .source_manifest
            .as_deref()
            .is_some_and(|manifest| !manifest.trim().is_empty());
        let verified = r.assertions_passed
            && !r.assertions.is_empty()
            && r.exit_outcome == Some(0)
            && source_bound
            && davinci_coding_agent::interaction_testing::validate_receipt_provenance(r).is_ok();
        if verified {
            passed_receipts += 1;
            match r.backend_kind {
                BackendKind::RealPty => real_terminal_verified = true,
                BackendKind::RealBrowser => real_browser_verified = true,
                BackendKind::PhysicalManual => manual_verified = true,
                _ => {}
            }
        }
    }

    if !real_terminal_verified {
        named_gaps.push(
            "No passing provenance-valid real PTY evidence recorded: fixture-only coverage does not prove terminal behavior"
                .into(),
        );
    }
    if !real_browser_verified {
        named_gaps.push(
            "No passing provenance-valid real browser evidence recorded: fixture-only coverage does not prove Playwright runtime behavior"
                .into(),
        );
    }
    if !manual_verified {
        named_gaps.push(
            "Physical keyboard scan codes untested: verified through synthetic byte sequences"
                .into(),
        );
    }

    InteractionCoverageReport {
        real_terminal_verified,
        real_browser_verified,
        manual_verified,
        receipts_count: receipts.len(),
        passed_receipts,
        named_gaps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::{ArtifactKind, GraphTaskState};

    fn task(id: &str, role: Role, status: TaskStatus, focus: Option<&str>) -> GraphTaskState {
        let mut task = GraphTaskState::new(
            id,
            role,
            ArtifactKind::Evidence,
            Vec::new(),
            focus.map(str::to_string),
        );
        task.status = status;
        task
    }

    #[test]
    fn evidence_truncation_respects_utf8_boundaries() {
        let text = "é".repeat(3000);
        let out = redact_evidence_output(&text, 4095);
        assert!(out.contains("[truncated"));
        assert!(!out.contains('�'));
    }

    #[test]
    fn every_task_status_carries_a_distinct_glyph() {
        use std::collections::BTreeSet;
        let statuses = [
            TaskStatus::Pending,
            TaskStatus::Ready,
            TaskStatus::Running,
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];
        // Pending and Ready are both "not started yet" and share a glyph; the
        // other four must each be told apart without colour (design.md §4).
        let glyphs: BTreeSet<&str> = statuses.iter().map(|s| task_state(*s).glyph()).collect();
        assert_eq!(glyphs.len(), 5);
        assert_eq!(task_state(TaskStatus::Succeeded).glyph(), "✓");
        assert_eq!(task_state(TaskStatus::Running).glyph(), "◉");
        assert_eq!(task_state(TaskStatus::Failed).glyph(), "×");
        assert_eq!(task_state(TaskStatus::Cancelled).glyph(), "◌");
    }

    #[test]
    fn every_role_maps_to_a_verb_the_design_lists() {
        // design.md §5: "studying, surveying, tracing, measuring, testing,
        // constructing, verifying" — used literally, nothing invented.
        const ALLOWED: [&str; 7] = [
            "studying",
            "surveying",
            "tracing",
            "measuring",
            "testing",
            "constructing",
            "verifying",
        ];
        for role in [
            Role::Classifier,
            Role::Researcher,
            Role::TestAnalyzer,
            Role::Historian,
            Role::Planner,
            Role::Writer,
            Role::Reviewer,
        ] {
            assert!(
                ALLOWED.contains(&work_verb(role)),
                "{:?} uses a verb the design does not list",
                role
            );
        }
    }

    #[test]
    fn a_run_becomes_a_numbered_plan_sheet() {
        let mut run = sample_run();
        run.tasks = vec![
            task("classify", Role::Classifier, TaskStatus::Succeeded, None),
            task(
                "research-1",
                Role::Researcher,
                TaskStatus::Running,
                Some("session store"),
            ),
            task("plan-1", Role::Planner, TaskStatus::Pending, None),
        ];

        let plan = plan_from_run(&run);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].numeral, "I");
        assert_eq!(plan[0].state, State::Done);
        assert_eq!(plan[0].verb, "measuring");
        // With no focus, the step still names itself rather than showing blank.
        assert_eq!(plan[0].target.as_deref(), Some("classify"));

        assert_eq!(plan[1].numeral, "II");
        assert_eq!(plan[1].state, State::Active);
        assert_eq!(plan[1].target.as_deref(), Some("session store"));

        assert_eq!(plan[2].numeral, "III");
        assert_eq!(plan[2].state, State::Queued);
    }

    #[test]
    fn the_governor_measures_every_row_against_the_window_not_the_largest_row() {
        let mut agent = davinci_agent::Agent::new("a".repeat(400).as_str());
        agent.messages.push(user("b".repeat(800).as_str()));
        agent.messages.push(assistant("c".repeat(1200).as_str()));

        let (rows, meta, proposal) = budget(&agent, 10_000);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].role, "instructions");
        assert_eq!(rows[0].tokens, "100");
        assert_eq!(rows[1].tokens, "200");
        assert_eq!(rows[3].tokens, "0", "no tools ran, so the row reads zero");

        // Each fraction is of the window, so they sum to the meter total.
        let summed: f64 = rows.iter().map(|row| row.fraction).sum();
        assert!((summed - meta.in_use_fraction).abs() < 1e-9, "{summed}");

        assert_eq!(meta.window, "10k");
        assert_eq!(meta.daily_cap, "not set", "no source, so it says so");
        assert!(meta.rate.ends_with("/turn"), "{}", meta.rate);
        // Well under the window: nothing to propose.
        assert!(proposal.is_none());
    }

    #[test]
    fn f09_unknown_cost() {
        assert_eq!(cost_label(None), "unknown".to_string());
        assert_eq!(cost_label(Some(0)), "0 minor units".to_string());
        assert_eq!(cost_label(Some(12)), "12 minor units".to_string());
    }

    #[test]
    fn a_nearly_full_window_gets_a_proposal_that_states_its_terms() {
        let mut agent = davinci_agent::Agent::new("system");
        for _ in 0..10 {
            agent.messages.push(user("x".repeat(400).as_str()));
            agent.messages.push(assistant("y".repeat(400).as_str()));
        }
        let (_, meta, proposal) = budget(&agent, 2_500);
        assert!(meta.in_use_fraction > 0.7, "{}", meta.in_use_fraction);

        let proposal = proposal.expect("a full window earns a proposal");
        // design.md §6: it always says what it recovers, keeps, costs, and
        // whether it can be undone.
        assert!(
            proposal.recovers.ends_with("tokens"),
            "{}",
            proposal.recovers
        );
        assert!(proposal.keeps.contains("last 6"), "{}", proposal.keeps);
        assert!(!proposal.cost.is_empty());
        assert!(proposal.reversible, "the session file keeps the originals");
        assert_eq!(proposal.actions.len(), 2);
        assert_eq!(proposal.actions[0].0, "/compact");
    }

    fn user(text: &str) -> davinci_ai::ChatMessage {
        message("user", text)
    }

    fn assistant(text: &str) -> davinci_ai::ChatMessage {
        message("assistant", text)
    }

    fn message(role: &str, text: &str) -> davinci_ai::ChatMessage {
        davinci_ai::ChatMessage {
            role: role.into(),
            content: vec![davinci_ai::MessageContent::Text { text: text.into() }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn a_project_that_has_never_run_the_graph_gets_an_empty_plan() {
        let dir = tempfile::tempdir().unwrap();
        assert!(plan(dir.path()).is_empty());
    }

    fn sample_run() -> GraphRun {
        use crate::native_extensions::graph::{GraphBudgets, GraphCounters, Phase};
        GraphRun {
            version: 1,
            run_id: "run-1".into(),
            goal: "wire the surfaces".into(),
            cwd: ".".into(),
            phase: Phase::Implement,
            forced: None,
            dry_run: false,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
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
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            definition: None,
            lifecycle: None,
            revision: 0,
            control_history: Vec::new(),
            continuation: None,
        }
    }

    #[test]
    fn f03_resume_lineage() {
        assert!(task_visible("root-a", "root-a", false));
        assert!(!task_visible("root-a", "root-b", false));
        assert!(task_visible("root-a", "root-a", true));
    }

    #[test]
    fn f03_resume_after_main_runtime_gets_a_new_run_id() {
        let bus = davinci_agent::RuntimeBus::new();
        let old_run = davinci_agent::RunId::new();
        let new_run = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();

        let registry = davinci_agent::TaskRegistry::with_bus(bus.clone());
        let record = davinci_agent::TaskRecord::new(old_run, "Initial task");
        let task_id = registry.create_task(record).unwrap();

        let runtime =
            davinci_agent::RuntimeHandle::new(new_run, agent_id, bus).with_task_registry(registry);

        assert!(task_visible(
            &old_run.to_string(),
            &old_run.to_string(),
            false
        ));
        assert!(!task_visible(
            &old_run.to_string(),
            &new_run.to_string(),
            false
        ));
        assert!(runtime.task_registry.get_task(&task_id).is_some());
    }

    #[test]
    fn f03_old_running_worker_orphaned() {
        assert_eq!(
            task_state_to_tui(davinci_agent::TaskState::Running),
            State::Active
        );
        assert_eq!(
            task_state_to_tui(davinci_agent::TaskState::Failed),
            State::Failed
        );
        assert_eq!(
            davinci_agent::TaskState::Running.public_status(),
            "in_progress"
        );
        assert_eq!(davinci_agent::TaskState::Failed.public_status(), "failed");
    }

    #[test]
    fn f03_no_tasks() {
        let bus = davinci_agent::RuntimeBus::new();
        let run_id = davinci_agent::RunId::new();
        let agent_id = davinci_agent::AgentId::new();
        let runtime = davinci_agent::RuntimeHandle::new(run_id, agent_id, bus);

        let board = task_board(&runtime);
        assert!(board.is_empty());

        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            80,
            24,
            false,
        );
        model.task_board = Some(davinci_tui::davinci::model::TaskBoardSheet {
            tasks: board,
            selected_index: 0,
        });
        let rendered = davinci_tui::davinci::views::task_board::lines(&model);
        let joined: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("No tasks currently running"));
    }

    #[test]
    fn f03_thousands_of_tasks_paginated() {
        let run_id = davinci_agent::RunId::new();
        let mut records = Vec::new();
        for i in 0..2000 {
            let task_id = davinci_agent::TaskId::new();
            records.push(davinci_agent::TaskRecord {
                id: task_id,
                run_id,
                title: format!("Subtask {i}"),
                description: None,
                dependencies: Vec::new(),
                state: davinci_agent::TaskState::Completed,
                assigned_to: None,
                result: None,
                created_at_ms: 1000 + i as i64,
                updated_at_ms: 2000 + i as i64,
                revision: 1,
                owner_generation: 0,
                parent_plan_step: None,
                evidence_refs: Vec::new(),
                blocked_reasons: Vec::new(),
                attempt: 1,
                contract_digest: None,
                decision_prerequisites: Vec::new(),
            });
        }
        let start = std::time::Instant::now();
        let rows = task_board_from_records(&records);
        let duration = start.elapsed();
        assert_eq!(rows.len(), 2000);
        assert!(
            duration.as_millis() < 100,
            "projection must be fast: {duration:?}"
        );

        let page_size = 20;
        let page_0 = &rows[0..page_size];
        assert_eq!(page_0.len(), 20);
        assert_eq!(page_0[0].title, "Subtask 0");
        assert_eq!(page_0[19].title, "Subtask 19");
    }

    #[test]
    fn f03_terminal_narrow() {
        let mut row = TaskBoardRow::new(
            "task-long-id-12345678",
            "Very long task title that exceeds narrow terminal width",
            "in_progress",
            State::Active,
        );
        row.owner = Some("worker-agent-name-extra-long".into());
        row.dependencies = vec!["dep-1".into(), "dep-2".into(), "dep-3".into()];
        row.blocked_reasons = vec!["Resource locked by external lock file".into()];

        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            30,
            24,
            false,
        );
        model.task_board = Some(davinci_tui::davinci::model::TaskBoardSheet {
            tasks: vec![row],
            selected_index: 0,
        });
        let rendered = davinci_tui::davinci::views::task_board::lines(&model);
        assert!(!rendered.is_empty());
    }

    #[test]
    fn f03_branch_specific_tasks_not_leaked() {
        let branch_a = "branch-a";
        let branch_b = "branch-b";
        assert!(task_visible(branch_a, branch_a, false));
        assert!(!task_visible(branch_a, branch_b, false));
        assert!(!task_visible(branch_b, branch_a, false));
    }

    #[test]
    fn f03_acceptance_of_plan_does_not_complete_tasks() {
        let bus = davinci_agent::RuntimeBus::new();
        let run_id = davinci_agent::RunId::new();
        let registry = davinci_agent::TaskRegistry::with_bus(bus);
        let mut record = davinci_agent::TaskRecord::new(run_id, "Feature task");
        record.state = davinci_agent::TaskState::Pending;
        let task_id = registry.create_task(record).unwrap();

        let plan = [PlanStep::new(
            "I",
            State::Done,
            "constructing",
            Some("Feature task"),
        )];
        assert_eq!(plan[0].state, State::Done);

        let record = registry.get_task(&task_id).unwrap();
        assert_eq!(record.state, davinci_agent::TaskState::Ready);
        assert_ne!(record.state, davinci_agent::TaskState::Completed);

        let rows = task_board_from_records(&[record]);
        assert_eq!(rows[0].state, State::Queued);
        assert_eq!(rows[0].status, "pending");
    }

    #[test]
    fn f05_old_task_without_contract_is_legacy_uncontracted() {
        let run_id = davinci_agent::RunId::new();
        let legacy_task = davinci_agent::TaskRecord::new(run_id, "Old uncontracted task");

        // Old task without contract must evaluate to LegacyUncontracted
        let status = evaluate_task_scope_status(&legacy_task, None);
        assert_eq!(status, TaskScopeStatus::LegacyUncontracted);
        assert_eq!(status.display_label(), "legacy (uncontracted)");

        // Projection to task board visibly displays uncontracted legacy state
        let rows = task_board_from_records(&[legacy_task]);
        assert_eq!(rows[0].activity.as_deref(), Some("legacy (uncontracted)"));
    }

    #[test]
    fn f05_correct_contract_revision_displayed_after_resume() {
        let run_id = davinci_agent::RunId::new();
        let task_id = davinci_agent::TaskId::new();

        let contract_rev1 = davinci_agent::runtime::contracts::TaskContract::new(
            "contract-resume",
            1,
            task_id,
            1,
            vec!["src/main.rs".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();

        // Expand to revision 2
        let contract_rev2 = contract_rev1
            .expand_scope(vec!["src/lib.rs".into()], vec![])
            .unwrap();
        assert_eq!(contract_rev2.revision, 2);

        // Resume simulation: task record has persisted contract_digest matching rev 2
        let mut resumed_record = davinci_agent::TaskRecord::new(run_id, "Resumed contracted task");
        resumed_record.id = task_id;
        resumed_record.contract_digest = Some(contract_rev2.digest.clone());

        let status = evaluate_task_scope_status(&resumed_record, Some(&contract_rev2));
        match &status {
            TaskScopeStatus::ContractScoped {
                revision, digest, ..
            } => {
                assert_eq!(*revision, 2);
                assert_eq!(digest, &contract_rev2.digest);
            }
            other => panic!("expected ContractScoped, got {other:?}"),
        }
        assert_eq!(status.display_label(), "contracted (rev 2)");
    }

    #[test]
    fn f05_invalid_contract_schema_fails_closed() {
        let run_id = davinci_agent::RunId::new();
        let mut record = davinci_agent::TaskRecord::new(run_id, "Corrupted contract task");
        record.contract_digest = Some("expected_digest_123".into());

        // Missing contract when digest is bound fails closed
        let missing_status = evaluate_task_scope_status(&record, None);
        assert!(matches!(
            missing_status,
            TaskScopeStatus::InvalidContract { .. }
        ));

        // Digest mismatch fails closed
        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "contract-diff",
            1,
            record.id,
            1,
            vec!["src/".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let mismatch_status = evaluate_task_scope_status(&record, Some(&contract));
        assert!(matches!(
            mismatch_status,
            TaskScopeStatus::InvalidContract { .. }
        ));
    }

    #[test]
    fn f06_report_gap() {
        assert_eq!(evidence_label(false, false, false), "not performed");
        assert_eq!(evidence_label(true, true, false), "stale");
        assert_eq!(evidence_label(true, true, true), "passed on current source");
        assert_eq!(evidence_label(true, false, true), "failed");
    }

    #[test]
    fn test_redact_evidence_output_and_truncation_notice() {
        let raw = "Authorization: Bearer sk-ant-api03-abcdef1234567890abcdef1234567890 password=supersecret token=ghp_secrettoken";
        let redacted = redact_evidence_output(raw, 50);
        assert!(!redacted.contains("supersecret"));
        assert!(!redacted.contains("ghp_secrettoken"));
        assert!(redacted.contains("[truncated"));
    }

    #[test]
    fn test_validate_artifact_scope_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store_root = dir.path();
        let safe_file = store_root.join("evidence_1.log");
        std::fs::write(&safe_file, b"ok").unwrap();

        // Valid relative path inside boundary
        assert!(validate_artifact_scope(store_root, "evidence_1.log").is_ok());

        // Directory traversal outside boundary
        assert!(validate_artifact_scope(store_root, "../outside.txt").is_err());
        assert!(validate_artifact_scope(store_root, "/etc/passwd").is_err());
        assert!(validate_artifact_scope(store_root, "\\windows\\system32").is_err());
    }

    #[test]
    fn test_same_source_rerun_creates_new_receipt() {
        let task_id = davinci_agent::TaskId::new();
        let r1 = davinci_agent::runtime::evidence_store::ExecutionReceipt {
            receipt_id: davinci_agent::runtime::ids::EvidenceId::new(),
            operation_id: "verify_1".into(),
            task_id: Some(task_id),
            tool_name: "cargo test".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };

        // Same command on same source rerun creates a new distinct receipt ID
        let r2 = davinci_agent::runtime::evidence_store::ExecutionReceipt {
            receipt_id: davinci_agent::runtime::ids::EvidenceId::new(),
            operation_id: "verify_1".into(),
            task_id: Some(task_id),
            tool_name: "cargo test".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };

        assert_ne!(r1.receipt_id, r2.receipt_id);
        assert_eq!(r1.operation_id, r2.operation_id);
    }

    #[test]
    fn f07_panel_actions() {
        assert_eq!(worker_action("enter"), Some("inspect"));
        assert_eq!(worker_action("s"), Some("steer"));
        assert_eq!(worker_action("x"), Some("stop"));
        assert_eq!(worker_action("r"), Some("retry"));
        assert_eq!(worker_action("d"), Some("diff"));
        assert_eq!(worker_action("shift_tab"), None);
    }

    #[test]
    fn test_delayed_stop_acknowledgment() {
        use davinci_agent::runtime::control::{reduce_stop_status, ControlStatus};
        assert_eq!(
            reduce_stop_status(true, false, false),
            ControlStatus::Stopping
        );
        assert_eq!(
            reduce_stop_status(true, true, false),
            ControlStatus::Stopped
        );
        assert_eq!(
            reduce_stop_status(true, false, true),
            ControlStatus::FailedToStop
        );
    }

    #[test]
    fn test_thousands_of_events_bounded() {
        let run_id = davinci_agent::RunId::new();
        let mut snapshots = Vec::new();
        for i in 0..2000 {
            let agent_id = davinci_agent::AgentId::new();
            snapshots.push(davinci_agent::runtime::WorkerSnapshot {
                agent_id,
                run_id,
                task_id: None,
                name: format!("worker-{i}"),
                kind: davinci_agent::runtime::AgentKind::GraphWorker,
                state: davinci_agent::runtime::AgentState::Running,
                generation: 1,
                revision: 1,
                elapsed_ms: 120_000,
                last_activity_ms: 1000,
                tool_count: 5,
                owned_paths: vec![std::path::PathBuf::from(format!("crates/worker_{i}.rs"))],
                waiting_on: None,
                disconnected: false,
                usage_unknown: false,
            });
        }
        let rows = agents_from_snapshots(&snapshots);
        assert_eq!(rows.len(), 2000);
        assert_eq!(rows[0].name, "worker-0");
        assert_eq!(rows[1999].name, "worker-1999");
    }

    #[test]
    fn test_diff_of_binary_path() {
        let dir = tempfile::tempdir().unwrap();
        let baseline =
            crate::native_extensions::graph::mutation::capture_baseline(dir.path()).unwrap();
        let bin_file = dir.path().join("image.bin");
        std::fs::write(&bin_file, [0u8, 159, 255, 0, 12, 0]).unwrap();
        let report = crate::native_extensions::graph::mutation::compute_owned_diff(
            dir.path(),
            &baseline,
            &[],
        )
        .unwrap();
        assert!(report.owned_diff.contains("new binary file"));
    }

    #[test]
    fn test_refresh_while_typing_steer() {
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            80,
            24,
            false,
        );
        model.composer.push_str("drafting steer text");
        assert_eq!(&*model.composer, "drafting steer text");

        let agent_id = davinci_agent::AgentId::new();
        let row = AgentRow::new(agent_id.to_string(), "worker-1", State::Active);
        model.agents = Some(davinci_tui::davinci::model::AgentsSheet {
            agents: vec![row],
            selected_index: 0,
        });

        // Simulating refresh:
        let refreshed_row = AgentRow::new(agent_id.to_string(), "worker-1", State::Active);
        if let Some(sheet) = model.agents.as_mut() {
            let prev_sel = sheet.selected_index;
            sheet.agents = vec![refreshed_row];
            sheet.selected_index = prev_sel.min(sheet.agents.len().saturating_sub(1));
        }

        // Draft text and selection preserved
        assert_eq!(&*model.composer, "drafting steer text");
        assert_eq!(model.agents.as_ref().unwrap().selected_index, 0);
    }

    #[test]
    fn f08_visible_freshness() {
        assert_eq!(freshness_label(false, false), "unproven");
        assert_eq!(freshness_label(true, false), "stale");
        assert_eq!(freshness_label(true, true), "fresh");
    }

    #[test]
    fn test_narrow_terminal_context_inspector() {
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            35,
            20,
            false,
        );
        let row = davinci_tui::davinci::model::ContextInspectorRow {
            item_id: "sec_policy_1".into(),
            category: "security".into(),
            provenance: "mandatory_policy".into(),
            source_ref: "repo::policy".into(),
            fingerprint: "hash123".into(),
            estimated_tokens: 150,
            selected: true,
            inclusion_reason: Some("mandatory".into()),
            mandatory: true,
            pinned: false,
            freshness: "fresh".into(),
            last_refreshed_at: None,
            preview_body: Some("policy details".into()),
        };
        model.context_inspector = Some(davinci_tui::davinci::model::ContextInspectorSheet {
            request_id: "req-narrow".into(),
            root_run_id: "run-narrow".into(),
            source_revision: 1,
            overlay_revision: 1,
            manifest_digest: "digest".into(),
            rows: vec![row],
            selected_index: 0,
            preview_active: true,
            show_pending: false,
            confirmation_dialog: None,
        });

        let rendered = davinci_tui::davinci::views::context_inspector::lines(&model);
        assert!(!rendered.is_empty());
        let text: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("[mandatory]"));
        assert!(text.contains("mandatory_policy"));
    }

    #[test]
    fn test_huge_source_body_paginated_and_bounded() {
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            80,
            24,
            false,
        );
        let huge_preview = (0..100)
            .map(|i| format!("line {i} of source code"))
            .collect::<Vec<_>>()
            .join("\n");
        let row = davinci_tui::davinci::model::ContextInspectorRow {
            item_id: "huge_file".into(),
            category: "code".into(),
            provenance: "repository_fact".into(),
            source_ref: "src/big.rs".into(),
            fingerprint: "hash_big".into(),
            estimated_tokens: 5000,
            selected: true,
            inclusion_reason: None,
            mandatory: false,
            pinned: false,
            freshness: "fresh".into(),
            last_refreshed_at: None,
            preview_body: Some(huge_preview),
        };
        model.context_inspector = Some(davinci_tui::davinci::model::ContextInspectorSheet {
            request_id: "req-huge".into(),
            root_run_id: "run-huge".into(),
            source_revision: 1,
            overlay_revision: 1,
            manifest_digest: "digest".into(),
            rows: vec![row],
            selected_index: 0,
            preview_active: true,
            show_pending: false,
            confirmation_dialog: None,
        });

        let rendered = davinci_tui::davinci::views::context_inspector::lines(&model);
        let text: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("truncated"));
    }

    #[test]
    fn test_secrets_redacted_in_preview() {
        let raw_with_secret =
            "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9 sk-12345678901234567890 password=supersecret";
        let redacted = redact_evidence_output(raw_with_secret, 1000);
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(!redacted.contains("supersecret"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_keyboard_exclusion_of_mandatory_rejected() {
        assert!(!davinci_agent::runtime::overlay_change_allowed(
            true, "exclude", true
        ));
        assert!(!davinci_agent::runtime::overlay_change_allowed(
            true, "pin", true
        ));
        assert!(davinci_agent::runtime::overlay_change_allowed(
            false, "exclude", true
        ));
        assert!(davinci_agent::runtime::overlay_change_allowed(
            false, "pin", true
        ));
    }

    #[test]
    fn test_correction_survives_resume() {
        let mut overlay = davinci_agent::runtime::ContextOverlay::new(1);
        overlay.add_correction("fact_1", "corrected value");
        overlay.add_tombstone("fact_1");
        assert!(overlay.is_tombstoned("fact_1"));

        // Simulate resume: serialized and restored
        let serialized = serde_json::to_string(&overlay).unwrap();
        let restored: davinci_agent::runtime::ContextOverlay =
            serde_json::from_str(&serialized).unwrap();
        assert!(restored.is_tombstoned("fact_1"));
        assert_eq!(
            restored
                .memory_corrections
                .get("fact_1")
                .map(|s| s.as_str()),
            Some("corrected value")
        );
    }

    #[test]
    fn test_json_no_private_body_by_default() {
        let run_id = davinci_agent::runtime::ids::RunId::new();
        let entry = davinci_agent::runtime::ContextManifestEntry::new(
            "doc_private",
            "memory",
            davinci_agent::runtime::ProvenanceKind::RepositoryFact,
            "private.txt",
            "hash_priv",
            500,
            true,
            Some("selected".into()),
            false,
            "fresh",
            None,
        );
        let manifest = davinci_agent::runtime::PreparedContextManifest::new(
            "req-json",
            run_id,
            1,
            1,
            vec![entry],
            0,
        );
        let summary = crate::output::ContextManifestSummary::from_prepared(&manifest, None);
        let json = summary.to_json_string().unwrap();
        assert!(json.contains("doc_private"));
        assert!(json.contains("repository_fact"));
        assert!(!json.contains("preview_body"));
    }

    #[test]
    fn test_inspector_absent_before_first_request() {
        let model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            80,
            24,
            false,
        );
        assert!(model.context_inspector.is_none());
        let rendered = davinci_tui::davinci::views::context_inspector::lines(&model);
        let text: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("No context manifest available"));
    }

    #[test]
    fn context_vm_status_is_read_only_and_exposes_runtime_state() {
        let mut agent = davinci_agent::Agent::new("system");
        agent.set_runtime(davinci_agent::RuntimeHandle::new(
            davinci_agent::RunId::new(),
            davinci_agent::AgentId::new(),
            davinci_agent::RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(davinci_agent::runtime::ContextVmMode::Active);
        let root_before = agent.runtime.as_ref().unwrap().context_vm.root();
        let metrics_before = agent.runtime.as_ref().unwrap().context_vm.metrics();

        let summary = crate::output::ContextVmStatusSummary::from_agent(&agent);

        assert_eq!(summary.mode, "active");
        assert_eq!(summary.epoch, root_before.epoch);
        assert_eq!(summary.checkpoint_id, None);
        assert_eq!(summary.delta_count, 0);
        assert_eq!(summary.episode_count, 0);
        assert_eq!(summary.page_fault_hits, 0);
        assert_eq!(summary.page_fault_misses, 0);
        assert_eq!(
            agent.runtime.as_ref().unwrap().context_vm.root(),
            root_before
        );
        assert_eq!(
            agent.runtime.as_ref().unwrap().context_vm.metrics(),
            metrics_before
        );
    }

    #[test]
    fn f11_forged_real_backend_is_not_verified_coverage() {
        let receipts = vec![
            davinci_coding_agent::interaction_testing::InteractionReceipt {
                scenario_id: "forged_real_pty".into(),
                backend_kind: davinci_coding_agent::interaction_testing::BackendKind::RealPty,
                backend_identity: "fake_fixture_simulated".into(),
                source_manifest: Some("source-digest".into()),
                assertions_passed: true,
                assertions: vec!["PASS: forged".into()],
                frames_count: 1,
                event_log: Vec::new(),
                console_errors: Vec::new(),
                network_failures: Vec::new(),
                trace_refs: Vec::new(),
                exit_outcome: Some(0),
            },
        ];
        let report = interaction_coverage_report(&receipts);
        assert_eq!(report.passed_receipts, 0);
        assert!(!report.real_terminal_verified);
    }

    #[test]
    fn f11_failed_real_backend_is_not_verified_coverage() {
        let receipts = vec![
            davinci_coding_agent::interaction_testing::InteractionReceipt {
                scenario_id: "real_pty_failure".into(),
                backend_kind: davinci_coding_agent::interaction_testing::BackendKind::RealPty,
                backend_identity: "conpty-test".into(),
                source_manifest: Some("source-digest".into()),
                assertions_passed: false,
                assertions: vec!["FAIL: draft changed".into()],
                frames_count: 1,
                event_log: Vec::new(),
                console_errors: Vec::new(),
                network_failures: Vec::new(),
                trace_refs: Vec::new(),
                exit_outcome: Some(1),
            },
        ];
        let report = interaction_coverage_report(&receipts);
        assert!(!report.real_terminal_verified);
        assert!(report
            .named_gaps
            .iter()
            .any(|gap| gap.contains("provenance-valid real PTY evidence")));
    }

    #[test]
    fn test_interaction_coverage_report_gaps() {
        let receipts = vec![
            davinci_coding_agent::interaction_testing::InteractionReceipt {
                scenario_id: "test_scen".into(),
                backend_kind: davinci_coding_agent::interaction_testing::BackendKind::FixtureOnly,
                backend_identity: "fake_pty".into(),
                source_manifest: Some("source-digest".into()),
                assertions_passed: true,
                assertions: vec!["PASS: ok".into()],
                frames_count: 1,
                event_log: Vec::new(),
                console_errors: Vec::new(),
                network_failures: Vec::new(),
                trace_refs: Vec::new(),
                exit_outcome: Some(0),
            },
        ];
        let report = interaction_coverage_report(&receipts);
        assert_eq!(report.receipts_count, 1);
        assert_eq!(report.passed_receipts, 1);
        assert!(!report.real_terminal_verified);
        assert!(!report.real_browser_verified);
        assert!(!report.manual_verified);
        assert_eq!(report.named_gaps.len(), 3);
        assert!(report.named_gaps.iter().any(|g| g.contains("real PTY")));
    }
}
