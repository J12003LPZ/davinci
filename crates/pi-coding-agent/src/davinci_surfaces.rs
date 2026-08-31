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

use pi_agent::Agent;
use pi_tui::davinci::model::{
    BudgetMeta, BudgetRow, Model, PlanStep, Proposal, RecallHit, RecallMeta,
};
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
        let tokens = pi_agent::estimate_context_tokens(std::slice::from_ref(message));
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
    let stats = pi_session::session_usage_stats(&store.entries);
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
    let recovers = pi_agent::estimate_context_tokens(&agent.messages[..older]);
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
    fn the_governor_measures_every_row_against_the_window_not_the_largest_row() {
        let mut agent = pi_agent::Agent::new("a".repeat(400).as_str());
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
    fn a_nearly_full_window_gets_a_proposal_that_states_its_terms() {
        let mut agent = pi_agent::Agent::new("system");
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

    fn user(text: &str) -> pi_ai::ChatMessage {
        message("user", text)
    }

    fn assistant(text: &str) -> pi_ai::ChatMessage {
        message("assistant", text)
    }

    fn message(role: &str, text: &str) -> pi_ai::ChatMessage {
        pi_ai::ChatMessage {
            role: role.into(),
            content: vec![pi_ai::MessageContent::Text { text: text.into() }],
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
