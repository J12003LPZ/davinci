//! Real data for the surfaces that have a source in this workspace.
//!
//! `davinci_sources.rs` covers what git and the session store know. This
//! covers what the native extensions know: the plan sheet (Disegno, `1c`)
//! reads the graph run's task list, and vector recall (Memoria, `2b`) reads
//! the vector memory index.
//!
//! Grafo (`2a`) still has no source — the `graph` extension is a multi-agent
//! run orchestrator, not a symbol index, and nothing in this workspace builds
//! one — so it stays on `pi_tui::davinci::fixtures` where it is obviously a
//! drawing rather than a claim.

use std::path::Path;
use std::time::Instant;

use pi_tui::davinci::model::{Model, PlanStep, RecallHit, RecallMeta};
use pi_tui::davinci::theme::State;
use pi_tui::davinci::views::disegno::roman;

use crate::native_extensions::graph::store;
use crate::native_extensions::graph::types::{GraphRun, Role, TaskStatus};
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
    let newest = store::list_runs(cwd).into_iter().next()?;
    store::load_run(cwd, &newest.run_id)
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
    pi_tui::davinci::views::chrome::thousands(value)
}

/// Fill the surfaces that have a source, leaving the ones that do not alone.
pub fn dress_from_extensions(model: &mut Model, cwd: &Path) {
    let plan = plan(cwd);
    if !plan.is_empty() {
        model.plan = plan;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{ArtifactKind, GraphTaskState};

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
    fn a_project_that_has_never_run_the_graph_gets_an_empty_plan() {
        let dir = tempfile::tempdir().unwrap();
        assert!(plan(dir.path()).is_empty());
    }

    fn sample_run() -> GraphRun {
        use crate::native_extensions::graph::types::{GraphBudgets, GraphCounters, Phase};
        GraphRun {
            version: 1,
            run_id: "run-1".into(),
            goal: "wire the surfaces".into(),
            cwd: ".".into(),
            phase: Phase::Implement,
            forced: None,
            dry_run: false,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: Vec::new(),
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
        }
    }
}
