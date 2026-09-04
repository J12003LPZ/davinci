//! `/graph` argument parsing and text rendering. Pure — the controller stays
//! free of presentation and the command layer stays thin glue.

use super::store::now_ms;
use super::types::{Complexity, GraphRun, GraphTaskState, TaskStatus, WorkerUsage};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedGraphArgs {
    pub goal: String,
    pub forced: Option<Complexity>,
    pub dry_run: bool,
}

/// Strip flags as whole tokens but keep the goal text verbatim otherwise:
/// a multi-line pasted brief must reach the classifier with its line structure
/// intact, because the classifier looks for numbered, separable deliverables.
pub fn parse_graph_args(args: &str) -> ParsedGraphArgs {
    let mut parsed = ParsedGraphArgs::default();
    let mut lines: Vec<String> = Vec::new();
    for line in args.lines() {
        let mut kept: Vec<&str> = Vec::new();
        for token in line.split_whitespace() {
            match token {
                "--simple" => parsed.forced = Some(Complexity::Trivial),
                "--complex" => parsed.forced = Some(Complexity::Complex),
                "--dry-run" => parsed.dry_run = true,
                _ => kept.push(token),
            }
        }
        lines.push(kept.join(" ").trim_end().to_string());
    }
    parsed.goal = lines.join("\n").trim().to_string();
    parsed
}

fn status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending | TaskStatus::Ready => "·",
        TaskStatus::Running => "◐",
        TaskStatus::Succeeded => "✓",
        TaskStatus::Failed => "✗",
        TaskStatus::Cancelled => "⊘",
    }
}

pub fn format_tokens(count: u64) -> String {
    match count {
        count if count >= 1_000_000 => format!("{:.1}M", count as f64 / 1_000_000.0),
        count if count >= 10_000 => format!("{}k", (count as f64 / 1000.0).round() as u64),
        count if count >= 1_000 => format!("{:.1}k", count as f64 / 1000.0),
        count => count.to_string(),
    }
}

pub fn format_elapsed(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes >= 60 {
        return format!("{}h{:02}m", minutes / 60, minutes % 60);
    }
    if minutes > 0 {
        return format!("{minutes}m{seconds:02}s");
    }
    format!("{seconds}s")
}

fn usage_suffix(usage: &WorkerUsage) -> String {
    let mut parts = Vec::new();
    if usage.input + usage.output > 0 {
        parts.push(format!(
            "{}↑ {}↓",
            format_tokens(usage.input),
            format_tokens(usage.output)
        ));
    }
    if usage.cost_usd > 0.0 {
        parts.push(format!("${:.2}", usage.cost_usd));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" {}", parts.join(" "))
    }
}

fn task_line(task: &GraphTaskState, now: u64) -> String {
    let mut line = format!(
        "{} {} ({}){}",
        status_icon(task.status),
        task.id,
        task.role,
        usage_suffix(&task.usage)
    );
    if let Some(started_at) = task.started_at {
        let ended = task.ended_at.unwrap_or(now);
        line.push_str(&format!(
            " {}",
            format_elapsed(ended.saturating_sub(started_at))
        ));
    }
    if task.status == TaskStatus::Running {
        if let Some(activity) = &task.last_activity {
            line.push_str(&format!(" — {activity}"));
        }
    }
    if let Some(error) = &task.error {
        line.push_str(&format!(" — {error}"));
    }
    line
}

pub fn render_run_lines(run: &GraphRun, now: u64) -> Vec<String> {
    let running = run
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Running)
        .count();
    let milestone = match (&run.milestones, run.current_milestone) {
        (Some(milestones), Some(current)) => {
            format!(" — milestone {current}/{}", milestones.len())
        }
        _ => String::new(),
    };
    let mut lines = vec![format!(
        "graph {} — {}{}{milestone} — {}↑ {}↓ — ${:.2} — {running} active / {} workers — {}",
        run.run_id,
        run.phase,
        if run.dry_run { " (dry-run)" } else { "" },
        format_tokens(run.total_input()),
        format_tokens(run.total_output()),
        run.counters.cost_usd,
        run.counters.workers_spawned,
        format_elapsed(now.saturating_sub(run.counters.started_at)),
    )];
    if let (Some(milestones), Some(current)) = (&run.milestones, run.current_milestone) {
        if let Some(text) = milestones.get(current.saturating_sub(1)) {
            lines.push(format!("◈ {}", text.chars().take(100).collect::<String>()));
        }
    }
    lines.extend(run.tasks.iter().map(|task| task_line(task, now)));
    if let Some(reason) = &run.blocked_reason {
        lines.push(format!("blocked: {reason}"));
    }
    lines
}

pub fn render_run_summary(run: &GraphRun) -> String {
    let duration_min = (run.updated_at.saturating_sub(run.counters.started_at)) as f64 / 60_000.0;
    let outcome = match &run.blocked_reason {
        Some(reason) => format!("{} — {reason}", run.phase),
        None => run.phase.to_string(),
    };
    let mut lines = vec![
        format!("## Graph run {}: {}", run.run_id, run.phase),
        String::new(),
        format!("- goal: {}", run.goal),
        format!("- outcome: {outcome}"),
        format!(
            "- cost: ${:.2} across {} workers, {duration_min:.1} min",
            run.counters.cost_usd, run.counters.workers_spawned
        ),
        format!(
            "- tokens: {} in / {} out",
            format_tokens(run.total_input()),
            format_tokens(run.total_output())
        ),
        format!(
            "- revision cycles: {}, replans: {}",
            run.counters.revision_cycles, run.counters.replans
        ),
        format!(
            "- state: .pi/graph/runs/{}/state.json (artifacts and logs beside it)",
            run.run_id
        ),
    ];
    if let Some(milestones) = &run.milestones {
        let reached = run.current_milestone.unwrap_or(0);
        let delivered_count = if run.phase == super::types::Phase::Done {
            milestones.len()
        } else {
            reached.saturating_sub(1)
        };
        lines.push(format!(
            "- milestones ({delivered_count}/{} delivered):",
            milestones.len()
        ));
        for (index, milestone) in milestones.iter().enumerate() {
            let delivered = run.phase == super::types::Phase::Done || index + 1 < reached;
            lines.push(format!(
                "  - [{}] {milestone}",
                if delivered { "x" } else { " " }
            ));
        }
    }
    if let Some(verification) = &run.verification {
        lines.push(format!(
            "- verification: {}",
            if verification.passed {
                "passed"
            } else {
                "FAILED"
            }
        ));
        for command in &verification.commands {
            lines.push(format!(
                "  - {}: {}",
                command.name,
                if command.skipped {
                    "skipped (command does not exist)".to_string()
                } else {
                    format!("exit {}", command.exit_code)
                }
            ));
        }
    }
    let tasks = run
        .tasks
        .iter()
        .map(|task| format!("{}:{}", task.id, task.status))
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("- tasks: {tasks}"));
    lines.join("\n")
}

pub fn render_now(run: &GraphRun) -> Vec<String> {
    render_run_lines(run, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{
        ArtifactKind, GraphBudgets, GraphCounters, Phase, Role,
    };

    fn run() -> GraphRun {
        GraphRun {
            version: 1,
            run_id: "r1".into(),
            goal: "do the thing".into(),
            cwd: ".".into(),
            phase: Phase::Implement,
            forced: None,
            dry_run: false,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![GraphTaskState::new(
                "implement-1",
                Role::Writer,
                ArtifactKind::PatchReport,
                vec![],
                None,
            )],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 1,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 1.5,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            updated_at: 120_000,
        }
    }

    #[test]
    fn flags_are_stripped_from_the_goal_text() {
        let parsed = parse_graph_args("--complex fix the parser --dry-run");
        assert_eq!(parsed.goal, "fix the parser");
        assert_eq!(parsed.forced, Some(Complexity::Complex));
        assert!(parsed.dry_run);
    }

    #[test]
    fn simple_forces_the_trivial_path() {
        let parsed = parse_graph_args("--simple rename a field");
        assert_eq!(parsed.forced, Some(Complexity::Trivial));
        assert!(!parsed.dry_run);
        assert_eq!(parsed.goal, "rename a field");
    }

    #[test]
    fn a_goal_without_flags_survives_verbatim() {
        let parsed = parse_graph_args("  make   it faster  ");
        assert_eq!(parsed.goal, "make it faster");
        assert_eq!(parsed.forced, None);
    }

    #[test]
    fn a_pasted_multi_line_brief_keeps_its_line_structure() {
        let parsed = parse_graph_args(
            "Bring the port to parity. --complex\n\
             1. fix the parser\n\
             2. fix the writer\n",
        );
        assert_eq!(
            parsed.goal,
            "Bring the port to parity.\n1. fix the parser\n2. fix the writer"
        );
        assert_eq!(parsed.forced, Some(Complexity::Complex));
    }

    #[test]
    fn a_blank_line_inside_a_brief_is_preserved() {
        let parsed = parse_graph_args("first paragraph\n\nsecond paragraph");
        assert_eq!(parsed.goal, "first paragraph\n\nsecond paragraph");
    }

    #[test]
    fn the_header_line_reports_phase_cost_and_worker_count() {
        let lines = render_run_lines(&run(), 65_000);
        assert!(lines[0].contains("graph r1 — implement"));
        assert!(lines[0].contains("$1.50"));
        assert!(lines[0].contains("1 workers"));
        assert!(lines[0].contains("1m05s"));
        assert!(lines[1].starts_with("· implement-1 (writer)"));
    }

    #[test]
    fn a_blocked_run_says_why_in_both_renderings() {
        let mut run = run();
        run.phase = Phase::Blocked;
        run.blocked_reason = Some("review failed".into());
        let lines = render_run_lines(&run, 1000);
        assert_eq!(lines.last().unwrap(), "blocked: review failed");
        let summary = render_run_summary(&run);
        assert!(summary.contains("- outcome: blocked — review failed"));
    }

    #[test]
    fn token_and_elapsed_formatting_stay_compact() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_500), "1.5k");
        assert_eq!(format_tokens(12_400), "12k");
        assert_eq!(format_tokens(2_000_000), "2.0M");
        assert_eq!(format_elapsed(5_000), "5s");
        assert_eq!(format_elapsed(65_000), "1m05s");
        assert_eq!(format_elapsed(3_700_000), "1h01m");
    }

    #[test]
    fn milestone_progress_is_checkboxed_in_the_summary() {
        let mut run = run();
        run.milestones = Some(vec!["one".into(), "two".into()]);
        run.current_milestone = Some(2);
        let summary = render_run_summary(&run);
        assert!(summary.contains("- milestones (1/2 delivered)"));
        assert!(summary.contains("  - [x] one"));
        assert!(summary.contains("  - [ ] two"));
    }
}
