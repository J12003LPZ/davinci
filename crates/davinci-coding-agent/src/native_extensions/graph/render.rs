//! `/graph` argument parsing and text rendering. Pure — the controller stays
//! free of presentation and the command layer stays thin glue.

use std::collections::HashMap;

use super::store::now_ms;
use super::types::{Complexity, GraphRun, GraphTaskState, TaskStatus, WorkerUsage};

use super::operations;
#[allow(unused_imports)]
pub use operations::*;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedGraphArgs {
    pub goal: String,
    pub forced: Option<Complexity>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphCommand {
    Current,
    Save {
        name: String,
        overwrite: bool,
    },
    RunSaved {
        name: String,
        params: HashMap<String, String>,
        dry_run: bool,
    },
    Goal(ParsedGraphArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GraphAdvancedCommand {
    Diff {
        revision: Option<u64>,
    },
    Explain {
        node_id: Option<String>,
    },
    DryRun {
        name: Option<String>,
    },
    Fork {
        node_id: String,
        strategy: Option<String>,
        authorized: bool,
    },
    Rewind {
        node_id: String,
        authorized: bool,
    },
    Verify,
    Budget {
        resource: Option<String>,
        value: Option<String>,
        expected_revision: Option<u64>,
        authorized: bool,
    },
    Export {
        name: Option<String>,
        overwrite: bool,
    },
}

#[allow(dead_code)]
pub fn parse_diff_args(args: &str) -> Result<Option<u64>, String> {
    let tokens = tokenize_args(args)?;
    let mut revision = None;
    for token in tokens {
        if token == "diff" {
            continue;
        }
        if let Ok(rev) = token.parse::<u64>() {
            revision = Some(rev);
        } else {
            return Err(format!("invalid revision '{token}' for /graph diff"));
        }
    }
    Ok(revision)
}

#[allow(dead_code)]
pub fn parse_explain_args(args: &str) -> Result<Option<String>, String> {
    let tokens = tokenize_args(args)?;
    let mut node_id = None;
    for token in tokens {
        if token == "explain" {
            continue;
        }
        if node_id.is_none() {
            node_id = Some(token);
        } else {
            return Err(format!("unexpected argument for /graph explain: '{token}'"));
        }
    }
    Ok(node_id)
}

#[allow(dead_code)]
pub fn parse_advanced_graph_command(args: &str) -> Result<Option<GraphAdvancedCommand>, String> {
    let trimmed = args.trim();
    let command = tokenize_args(trimmed)?;
    let Some(first) = command.first().cloned() else {
        return Ok(None);
    };
    if first == "diff" {
        Ok(Some(GraphAdvancedCommand::Diff {
            revision: parse_diff_args(trimmed)?,
        }))
    } else if first == "explain" {
        Ok(Some(GraphAdvancedCommand::Explain {
            node_id: parse_explain_args(trimmed)?,
        }))
    } else if first == "dry-run" {
        let tokens = command;
        let mut name = None;
        for token in tokens.into_iter().skip(1) {
            if name.is_none() {
                name = Some(token);
            } else {
                return Err(format!("unexpected argument for /graph dry-run: '{token}'"));
            }
        }
        Ok(Some(GraphAdvancedCommand::DryRun { name }))
    } else if first == "fork" {
        let tokens = command;
        if tokens.len() < 2 {
            return Err(
                "missing node argument for /graph fork: usage: /graph fork <node> [strategy]"
                    .into(),
            );
        }
        let node_id = tokens[1].clone();
        let mut strategy = None;
        let mut authorized = false;
        for token in tokens.into_iter().skip(2) {
            if token == "--authorize" {
                authorized = true;
            } else if token.starts_with("--") {
                return Err(format!("unknown option for /graph fork: '{token}'"));
            } else if strategy.is_none() {
                strategy = Some(token);
            } else {
                return Err(format!("unexpected argument for /graph fork: '{token}'"));
            }
        }
        Ok(Some(GraphAdvancedCommand::Fork {
            node_id,
            strategy,
            authorized,
        }))
    } else if first == "rewind" {
        let tokens = command;
        if tokens.len() < 2 {
            return Err(
                "missing node argument for /graph rewind: usage: /graph rewind <node>".into(),
            );
        }
        let node_id = tokens[1].clone();
        let mut authorized = false;
        for token in tokens.into_iter().skip(2) {
            if token == "--authorize" {
                authorized = true;
            } else {
                return Err(format!("unexpected argument for /graph rewind: '{token}'"));
            }
        }
        Ok(Some(GraphAdvancedCommand::Rewind {
            node_id,
            authorized,
        }))
    } else if first == "verify" {
        if command.len() > 1 {
            return Err("unexpected argument for /graph verify".into());
        }
        Ok(Some(GraphAdvancedCommand::Verify))
    } else if first == "budget" {
        let mut resource = None;
        let mut value = None;
        let mut expected_revision = None;
        let mut authorized = false;
        let mut positional = Vec::new();
        let mut saw_set = false;
        let mut tokens = command.into_iter().skip(1);
        while let Some(token) = tokens.next() {
            match token.as_str() {
                "set" if positional.is_empty() => saw_set = true,
                "--authorize" => authorized = true,
                "--revision" => {
                    let revision = tokens
                        .next()
                        .ok_or_else(|| "missing revision after --revision".to_string())?;
                    expected_revision =
                        Some(revision.parse::<u64>().map_err(|_| {
                            format!("invalid revision '{revision}' for /graph budget")
                        })?);
                }
                _ if token.starts_with("--") => {
                    return Err(format!("unknown option for /graph budget: '{token}'"));
                }
                _ => positional.push(token),
            }
        }
        if positional.len() > 2 {
            return Err(
                "Usage: /graph budget [set] <resource> <value> [--revision N] [--authorize]".into(),
            );
        }
        if positional.is_empty() && (saw_set || expected_revision.is_some() || authorized) {
            return Err(
                "Usage: /graph budget [set] <resource> <value> [--revision N] [--authorize]".into(),
            );
        }
        if let Some(item) = positional.first() {
            resource = Some(item.clone());
        }
        if let Some(item) = positional.get(1) {
            value = Some(item.clone());
        }
        if value.is_none() && resource.is_some() {
            return Err(
                "budget inspection takes no resource; updates require <resource> <value>".into(),
            );
        }
        Ok(Some(GraphAdvancedCommand::Budget {
            resource,
            value,
            expected_revision,
            authorized,
        }))
    } else if first == "export" {
        let mut name = None;
        let mut overwrite = false;
        for token in command.into_iter().skip(1) {
            if token == "--overwrite" {
                overwrite = true;
            } else if name.is_none() {
                if !super::definitions::safe_graph_name(&token) {
                    return Err(format!("invalid export name '{token}'"));
                }
                name = Some(token);
            } else {
                return Err(format!("unexpected argument for /graph export: '{token}'"));
            }
        }
        if name.is_none() && overwrite {
            return Err("/graph export --overwrite requires an export name".into());
        }
        Ok(Some(GraphAdvancedCommand::Export { name, overwrite }))
    } else {
        Ok(None)
    }
}

pub fn graph_command_kind(args: &str) -> &'static str {
    let args = args.trim();
    if args.is_empty() {
        return "current";
    }
    if args.starts_with("-- ") {
        return "goal";
    }
    match args.split_whitespace().next() {
        Some("save") => "save",
        Some("run") => "run",
        _ => "goal",
    }
}

pub fn tokenize_args(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for c in input.chars() {
        match c {
            '"' | '\'' if in_quote == Some(c) => {
                in_quote = None;
            }
            '"' | '\'' if in_quote.is_none() => {
                in_quote = Some(c);
            }
            c if c.is_whitespace() && in_quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if let Some(q) = in_quote {
        return Err(format!("unclosed quote `{q}` in arguments"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

pub fn parse_graph_command(args: &str) -> Result<GraphCommand, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(GraphCommand::Current);
    }
    if trimmed.starts_with("-- ") {
        let goal_text = trimmed.strip_prefix("-- ").unwrap().trim();
        return Ok(GraphCommand::Goal(parse_graph_args(goal_text)));
    }
    let kind = graph_command_kind(trimmed);
    match kind {
        "save" => {
            let tokens = tokenize_args(trimmed)?;
            let mut name: Option<String> = None;
            let mut overwrite = false;
            for token in tokens.into_iter().skip(1) {
                if token == "--overwrite" {
                    overwrite = true;
                } else if name.is_none() {
                    if !super::definitions::safe_graph_name(&token) {
                        return Err(format!("invalid graph name '{token}'"));
                    }
                    name = Some(token);
                } else {
                    return Err(format!("unexpected argument for /graph save: '{token}'"));
                }
            }
            let Some(name) = name else {
                return Err("Usage: /graph save <name> [--overwrite]".into());
            };
            Ok(GraphCommand::Save { name, overwrite })
        }
        "run" => {
            let tokens = tokenize_args(trimmed)?;
            let mut name: Option<String> = None;
            let mut dry_run = false;
            let mut params = HashMap::new();
            for token in tokens.into_iter().skip(1) {
                if token == "--dry-run" {
                    dry_run = true;
                } else if name.is_none() && !token.contains('=') {
                    if !super::definitions::safe_graph_name(&token) {
                        return Err(format!("invalid graph name '{token}'"));
                    }
                    name = Some(token);
                } else if let Some((k, v)) = token.split_once('=') {
                    if !super::definitions::is_safe_param_name(k) {
                        return Err(format!("invalid parameter name '{k}'"));
                    }
                    if !super::definitions::is_safe_param_value(v) {
                        return Err(format!("unsafe parameter value for '{k}'"));
                    }
                    params.insert(k.to_string(), v.to_string());
                } else {
                    return Err(format!("unexpected argument for /graph run: '{token}'"));
                }
            }
            let Some(name) = name else {
                return Err("Usage: /graph run <name> [param=value ...] [--dry-run]".into());
            };
            Ok(GraphCommand::RunSaved {
                name,
                params,
                dry_run,
            })
        }
        "goal" => Ok(GraphCommand::Goal(parse_graph_args(trimmed))),
        _ => Ok(GraphCommand::Current),
    }
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
    let ecosystem = run.ecosystem_stats.render_compact_lines();
    if !ecosystem.is_empty() {
        lines.extend(ecosystem);
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
            if let Some(progress) = &verification.progress {
                format!(
                    "running {}/{}: {}",
                    progress.index, progress.total, progress.command
                )
            } else if verification.passed {
                "passed".into()
            } else {
                "FAILED".into()
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
    let ecosystem = run.ecosystem_stats.render_compact_lines();
    if !ecosystem.is_empty() {
        lines.push("- ecosystem:".to_string());
        for line in ecosystem {
            lines.push(format!("  - {line}"));
        }
    }
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
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
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
            ecosystem_stats: Default::default(),
            updated_at: 120_000,
            lifecycle: None,
            revision: 0,
            control_history: Vec::new(),
            continuation: None,
        }
    }

    #[test]
    fn summary_distinguishes_active_verification_from_failure() {
        let mut run = run();
        run.verification = Some(super::super::types::VerificationResult {
            commands: vec![],
            passed: false,
            progress: Some(super::super::types::VerificationProgress {
                name: "test".into(),
                command: "cargo test".into(),
                index: 2,
                total: 3,
                started_at: 0,
            }),
        });
        let summary = render_run_summary(&run);
        assert!(summary.contains("verification: running 2/3: cargo test"));
        assert!(!summary.contains("FAILED"));
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

    #[test]
    fn f12_reserved_word_escape() {
        assert_eq!(graph_command_kind(""), "current");
        assert_eq!(graph_command_kind("save security-audit"), "save");
        assert_eq!(graph_command_kind("run security-audit"), "run");
        assert_eq!(graph_command_kind("-- save the broken file"), "goal");
        assert_eq!(graph_command_kind("repair OAuth"), "goal");
    }

    #[test]
    fn test_parse_graph_command_bare_current() {
        assert_eq!(parse_graph_command("").unwrap(), GraphCommand::Current);
        assert_eq!(parse_graph_command("   ").unwrap(), GraphCommand::Current);
    }

    #[test]
    fn test_parse_graph_command_save() {
        let cmd = parse_graph_command("save security-audit").unwrap();
        assert_eq!(
            cmd,
            GraphCommand::Save {
                name: "security-audit".into(),
                overwrite: false,
            }
        );

        let cmd = parse_graph_command("save security-audit --overwrite").unwrap();
        assert_eq!(
            cmd,
            GraphCommand::Save {
                name: "security-audit".into(),
                overwrite: true,
            }
        );
    }

    #[test]
    fn test_parse_graph_command_save_validation() {
        // Missing name
        let err = parse_graph_command("save").unwrap_err();
        assert!(err.contains("Usage: /graph save <name>"));

        // Invalid name
        let err = parse_graph_command("save ../escaping").unwrap_err();
        assert!(err.contains("invalid graph name"));
    }

    #[test]
    fn test_parse_graph_command_run() {
        let cmd = parse_graph_command("run security-audit").unwrap();
        match cmd {
            GraphCommand::RunSaved {
                name,
                params,
                dry_run,
            } => {
                assert_eq!(name, "security-audit");
                assert!(params.is_empty());
                assert!(!dry_run);
            }
            _ => panic!("expected RunSaved"),
        }

        let cmd = parse_graph_command("run security-audit scope=crates/agent --dry-run").unwrap();
        match cmd {
            GraphCommand::RunSaved {
                name,
                params,
                dry_run,
            } => {
                assert_eq!(name, "security-audit");
                assert_eq!(params.get("scope").unwrap(), "crates/agent");
                assert!(dry_run);
            }
            _ => panic!("expected RunSaved"),
        }
    }

    #[test]
    fn test_parse_graph_command_run_paths_with_spaces_and_quotes() {
        let cmd =
            parse_graph_command("run my-pipeline path=\"src/my dir/file.rs\" workers=2").unwrap();
        match cmd {
            GraphCommand::RunSaved {
                name,
                params,
                dry_run,
            } => {
                assert_eq!(name, "my-pipeline");
                assert_eq!(params.get("path").unwrap(), "src/my dir/file.rs");
                assert_eq!(params.get("workers").unwrap(), "2");
                assert!(!dry_run);
            }
            _ => panic!("expected RunSaved"),
        }
    }

    #[test]
    fn test_parse_graph_command_run_validation() {
        // Missing name
        let err = parse_graph_command("run").unwrap_err();
        assert!(err.contains("Usage: /graph run <name>"));

        // Unsafe parameter value (shell expansion)
        let err = parse_graph_command("run pipe target=$(whoami)").unwrap_err();
        assert!(err.contains("unsafe parameter value"));

        // Invalid parameter name
        let err = parse_graph_command("run pipe bad:param=1").unwrap_err();
        assert!(err.contains("invalid parameter name"));
    }

    #[test]
    fn test_parse_graph_command_goals_and_escapes() {
        // Normal goal
        let cmd = parse_graph_command("repair OAuth login flow --dry-run").unwrap();
        assert_eq!(
            cmd,
            GraphCommand::Goal(ParsedGraphArgs {
                goal: "repair OAuth login flow".into(),
                forced: None,
                dry_run: true,
            })
        );

        // Escaped reserved word goal
        let cmd = parse_graph_command("-- save the broken file").unwrap();
        assert_eq!(
            cmd,
            GraphCommand::Goal(ParsedGraphArgs {
                goal: "save the broken file".into(),
                forced: None,
                dry_run: false,
            })
        );

        // Multiline goal preserved
        let brief = "first line\nsecond line\nthird line";
        let cmd = parse_graph_command(brief).unwrap();
        match cmd {
            GraphCommand::Goal(parsed) => {
                assert_eq!(parsed.goal, brief);
            }
            _ => panic!("expected Goal"),
        }
    }

    #[test]
    fn test_parse_graph_command_diff_and_explain() {
        assert_eq!(
            parse_advanced_graph_command("diff").unwrap(),
            Some(GraphAdvancedCommand::Diff { revision: None })
        );
        assert_eq!(
            parse_advanced_graph_command("diff 3").unwrap(),
            Some(GraphAdvancedCommand::Diff { revision: Some(3) })
        );
        assert!(parse_advanced_graph_command("diff abc").is_err());

        assert_eq!(
            parse_advanced_graph_command("explain").unwrap(),
            Some(GraphAdvancedCommand::Explain { node_id: None })
        );
        assert_eq!(
            parse_advanced_graph_command("explain writer-1").unwrap(),
            Some(GraphAdvancedCommand::Explain {
                node_id: Some("writer-1".into())
            })
        );
        assert!(parse_advanced_graph_command("explain writer-1 extra").is_err());
    }

    #[test]
    fn f14_parse_budget_and_export_commands() {
        assert_eq!(
            parse_advanced_graph_command("budget").unwrap(),
            Some(GraphAdvancedCommand::Budget {
                resource: None,
                value: None,
                expected_revision: None,
                authorized: false,
            })
        );
        assert_eq!(
            parse_advanced_graph_command("budget set max-cost-usd 4.5 --revision 7 --authorize")
                .unwrap(),
            Some(GraphAdvancedCommand::Budget {
                resource: Some("max-cost-usd".into()),
                value: Some("4.5".into()),
                expected_revision: Some(7),
                authorized: true,
            })
        );
        assert_eq!(
            parse_advanced_graph_command("export").unwrap(),
            Some(GraphAdvancedCommand::Export {
                name: None,
                overwrite: false,
            })
        );
        assert_eq!(
            parse_advanced_graph_command("export release --overwrite").unwrap(),
            Some(GraphAdvancedCommand::Export {
                name: Some("release".into()),
                overwrite: true,
            })
        );
        assert!(parse_advanced_graph_command("budget set").is_err());
        assert!(parse_advanced_graph_command("export ../secret").is_err());
        assert!(parse_advanced_graph_command("export --overwrite").is_err());
    }

    #[test]
    fn f14_all_advanced_commands_have_exact_argument_boundaries() {
        assert!(matches!(
            parse_advanced_graph_command("dry-run").unwrap(),
            Some(GraphAdvancedCommand::DryRun { name: None })
        ));
        assert!(matches!(
            parse_advanced_graph_command("fork writer-1 repair-minimal").unwrap(),
            Some(GraphAdvancedCommand::Fork {
                authorized: false,
                ..
            })
        ));
        assert!(matches!(
            parse_advanced_graph_command("rewind writer-1").unwrap(),
            Some(GraphAdvancedCommand::Rewind {
                authorized: false,
                ..
            })
        ));
        assert!(matches!(
            parse_advanced_graph_command("fork writer-1 --authorize").unwrap(),
            Some(GraphAdvancedCommand::Fork {
                authorized: true,
                ..
            })
        ));
        assert!(matches!(
            parse_advanced_graph_command("rewind writer-1 --authorize").unwrap(),
            Some(GraphAdvancedCommand::Rewind {
                authorized: true,
                ..
            })
        ));
        assert!(matches!(
            parse_advanced_graph_command("verify").unwrap(),
            Some(GraphAdvancedCommand::Verify)
        ));
        assert!(parse_advanced_graph_command("fork writer-1 extra trailing").is_err());
        assert!(parse_advanced_graph_command("rewind writer-1 extra").is_err());
        assert!(parse_advanced_graph_command("fork writer-1 --unknown").is_err());
        assert!(parse_advanced_graph_command("diffuse").unwrap().is_none());
    }
}
