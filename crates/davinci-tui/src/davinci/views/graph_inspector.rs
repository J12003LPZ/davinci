//! Only public, explicitly exposed execution facts enter the details panel.
use super::graph_canvas::{agent_name, bucket_color, progress_text, status_label};
use crate::davinci::model::{GraphBucket, GraphRunSheet, GraphTask, Model};
use crate::davinci::{theme::State, ui};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A public field containing a private-reasoning marker is withheld as a whole.
/// Filtering before wrapping prevents a long marker being split across rows.
pub fn public_text(value: &str) -> String {
    let lower = value.to_lowercase();
    if [
        "thinking:",
        "reasoning:",
        "<thought",
        "<thinking",
        "<reasoning",
        "chain-of-thought",
        "private prompt",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return "[private content omitted]".into();
    }
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

/// Width of the label column: `Revisions:` plus two cells of air.
const LABEL: u16 = 12;
const ACTIVITY_ROWS: usize = 3;
const MESSAGE_ROWS: usize = 4;
const NEXT_STEPS: usize = 4;

/// The key reference `?` shows in place of the details.
pub const HELP: [(&str, &str); 14] = [
    ("↑↓←→", "select an agent"),
    ("Enter", "inspect / back"),
    ("Tab", "next filter"),
    ("Shift+Tab", "previous filter"),
    ("i", "message the main conversation"),
    ("p", "pause or resume the run"),
    ("x", "stop the selected agent"),
    ("r", "retry the selected agent"),
    ("s", "resume a stopped run"),
    ("d", "diff of the run's changes"),
    ("f", "follow live work"),
    ("v", "focus on the selection's path"),
    ("g", "show the goal"),
    ("PgUp/PgDn", "scroll details or pan"),
];

pub fn inspector_lines(
    model: &Model,
    selected: Option<&str>,
    width: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    if max_rows == 0 || model.graph_run.is_none() {
        return Vec::new();
    }
    let facts = fitted_facts(model, selected, width, max_rows);
    let count = facts.len();
    let fits = count < max_rows as usize;
    let room = if fits {
        count
    } else {
        max_rows.saturating_sub(2) as usize
    };
    let offset = model
        .graph_canvas
        .inspector_scroll
        .min(count.saturating_sub(room));
    let mut rows = vec![panel_title(model, width)];
    rows.extend(facts.into_iter().skip(offset).take(room));
    if !fits && max_rows > 1 {
        rows.push(Line::from(ui::span(
            ui::clip_ellipsis(
                &format!(
                    "PgUp/PgDn · {}–{} / {count}",
                    if count == 0 { 0 } else { offset + 1 },
                    (offset + room).min(count)
                ),
                width,
            ),
            model.theme.muted,
        )));
    }
    rows.truncate(max_rows as usize);
    rows
}

pub fn page(model: &mut Model, width: u16, height: u16, delta: isize) {
    let selected = model
        .graph_run
        .as_ref()
        .and_then(|r| r.selected_node_id.as_deref());
    let maximum = fitted_facts(model, selected, width, height)
        .len()
        .saturating_sub(height.saturating_sub(2) as usize);
    model.graph_canvas.inspector_scroll = model
        .graph_canvas
        .inspector_scroll
        .min(maximum)
        .saturating_add_signed(delta)
        .min(maximum);
}

/// Sections are separated by a blank row when everything fits; a panel that
/// would scroll anyway drops the separators first.
fn fitted_facts(
    model: &Model,
    selected: Option<&str>,
    width: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    let spaced = inspector_facts(model, selected, width, true);
    if spaced.len() < max_rows as usize {
        spaced
    } else {
        inspector_facts(model, selected, width, false)
    }
}

fn panel_title(model: &Model, width: u16) -> Line<'static> {
    let th = &model.theme;
    let run = model.graph_run.as_ref().unwrap();
    let (title, hint) = if model.graph_canvas.show_help {
        ("Keys", " · ? close")
    } else if model.graph_canvas.inspecting_goal {
        ("Goal", " · g agent details")
    } else if run.inspecting_node {
        ("Agent details", " · Enter back")
    } else {
        ("Agent details", " · Enter to focus")
    };
    Line::from(ui::truncate_run(
        vec![
            Span::styled(
                title.to_string(),
                Style::default().fg(th.text).add_modifier(Modifier::BOLD),
            ),
            ui::span(hint, th.muted),
        ],
        width,
    ))
}

struct Facts<'a> {
    rows: Vec<Line<'static>>,
    width: u16,
    model: &'a Model,
    spaced: bool,
}

impl Facts<'_> {
    fn blank(&mut self) {
        if self.rows.last().is_some_and(|row| row.width() > 0) {
            self.rows.push(Line::default());
        }
    }

    fn heading(&mut self, text: &str) {
        if self.spaced {
            self.blank();
        }
        let th = &self.model.theme;
        self.rows.push(Line::from(Span::styled(
            ui::clip_ellipsis(text, self.width),
            Style::default().fg(th.primary).add_modifier(Modifier::BOLD),
        )));
    }

    fn plain(&mut self, text: &str, color: Color) {
        for row in ui::wrap(&public_text(text), self.width) {
            self.rows.push(Line::from(ui::span(row, color)));
        }
    }

    /// `Label:      value`, the value wrapping under itself.
    fn fact(&mut self, label: &str, value: &str, style: Style) {
        let value = public_text(value);
        if value.trim().is_empty() {
            return;
        }
        let th = &self.model.theme;
        // Narrow panels (the bottom drawer, the list view) cannot spare an
        // aligned label column; there the label runs inline.
        let label_width = if self.width < 48 {
            unicode_width::UnicodeWidthStr::width(label) as u16 + 2
        } else {
            LABEL
        }
        .min(self.width / 2);
        let room = self.width.saturating_sub(label_width).max(1);
        for (i, row) in ui::wrap(&value, room).into_iter().enumerate() {
            let lead = if i == 0 {
                ui::clip(&format!("{label}:"), label_width)
            } else {
                String::new()
            };
            self.rows.push(Line::from(vec![
                ui::span(format!("{lead:<w$}", w = label_width as usize), th.muted),
                Span::styled(row, style),
            ]));
        }
    }

    /// A bordered box of at most `max` wrapped rows, the newest last.
    fn boxed(&mut self, lines: &[String], max: usize) {
        let th = &self.model.theme;
        let frame = Style::default().fg(th.border);
        if self.width < 7 {
            for line in lines {
                self.plain(line, th.text);
            }
            return;
        }
        // One column of air on the right, clear of the terminal edge.
        let outer = self.width - 1;
        let inner = outer - 4;
        let mut body: Vec<String> = lines
            .iter()
            .flat_map(|line| ui::wrap(&public_text(line), inner))
            .collect();
        if body.len() > max {
            // Keep the head of the text; mark what was cut.
            body.truncate(max);
            if let Some(last) = body.last_mut() {
                *last = ui::clip_ellipsis(&format!("{last}…"), inner);
            }
        }
        let rule = "─".repeat(outer as usize - 2);
        self.rows
            .push(Line::from(Span::styled(format!("┌{rule}┐"), frame)));
        for row in body {
            let pad = inner.saturating_sub(ui::run_width(&[Span::raw(row.clone())]));
            self.rows.push(Line::from(vec![
                Span::styled("│ ", frame),
                ui::span(row, th.text),
                Span::raw(" ".repeat(pad as usize)),
                Span::styled(" │", frame),
            ]));
        }
        self.rows
            .push(Line::from(Span::styled(format!("└{rule}┘"), frame)));
    }
}

fn inspector_facts(
    model: &Model,
    selected: Option<&str>,
    width: u16,
    spaced: bool,
) -> Vec<Line<'static>> {
    let Some(run) = &model.graph_run else {
        return Vec::new();
    };
    let th = &model.theme;
    let mut out = Facts {
        rows: Vec::new(),
        width,
        model,
        spaced,
    };
    let text = Style::default().fg(th.text);
    if model.graph_canvas.show_help {
        out.blank();
        for (key, action) in HELP {
            out.fact(key, action, text);
        }
        return out.rows;
    }
    if model.graph_canvas.inspecting_goal {
        out.blank();
        out.fact("Status", &run.lifecycle, text);
        out.fact("Phase", &run.phase, text);
        out.heading("Original prompt");
        out.plain(&run.goal, th.text);
        out.heading("Progress");
        for (task, bucket) in run.tasks.iter().zip(run.buckets()) {
            out.rows.push(Line::from(vec![
                ui::span(
                    format!("{} ", task.state.glyph()),
                    bucket_color(th, bucket, Some(task)),
                ),
                ui::span(
                    ui::clip_ellipsis(
                        &public_text(&format!(
                            "{} · {}",
                            agent_name(task),
                            status_label(bucket, task)
                        )),
                        width.saturating_sub(2),
                    ),
                    th.text,
                ),
            ]));
        }
        return out.rows;
    }
    let task = selected
        .and_then(|id| run.tasks.iter().find(|t| t.id == id))
        .or_else(|| run.tasks.iter().find(|t| t.state == State::Active));
    if let Some(group) = &model.graph_canvas.selected_group {
        out.blank();
        out.fact("Group", group.strip_prefix("fold:").unwrap_or(group), text);
        out.plain("Completed agents that share every dependency.", th.muted);
        out.plain("Enter expands its real members.", th.muted);
    } else if let Some(task) = task {
        agent_facts(&mut out, run, task);
    } else {
        out.blank();
        out.plain("Arrows select an agent; Enter inspects.", th.muted);
    }
    run_facts(&mut out, run, model.graph_canvas.inspecting_goal);
    out.rows
}

fn agent_facts(out: &mut Facts<'_>, run: &GraphRunSheet, task: &GraphTask) {
    let th = &out.model.theme;
    let bucket = run.bucket(task);
    let tone = bucket_color(th, bucket, Some(task));
    let text = Style::default().fg(th.text);
    out.blank();
    out.rows.push(Line::from(Span::styled(
        ui::clip_ellipsis(
            public_text(&format!(
                "{} {} {}",
                task.state.glyph(),
                task.id,
                if task.role == task.id { "" } else { &task.role }
            ))
            .trim_end(),
            out.width,
        ),
        Style::default().fg(tone).add_modifier(Modifier::BOLD),
    )));
    let status_style = Style::default()
        .fg(tone)
        .add_modifier(if bucket == GraphBucket::Working {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
    out.fact("Status", status_label(bucket, task), status_style);
    out.fact("Task", &task.title, text);
    out.fact("Project", &run.project, text);
    if let Some(branch) = &task.branch {
        out.fact("Branch", branch, text);
    } else if task.worktree.is_none() && !run.project.is_empty() && run.project == out.model.cwd {
        // A worker without its own worktree runs in the project checkout, so
        // the session's branch is the worker's branch.
        out.fact("Branch", &out.model.branch, text);
    }
    if let Some(worktree) = &task.worktree {
        out.fact("Worktree", worktree, text);
    }
    if let Some(model) = &task.model {
        out.fact("Model", model, text);
    }
    out.fact("Tokens", &task.tokens, text);
    out.fact("Started", &task.started, text);
    out.fact("Elapsed", &task.elapsed, text);
    // The agent's own spend; the run's spend against its cap heads the
    // command center and the status bar.
    out.fact("Cost", &task.cost, text);
    out.fact("Revisions", &run.revisions, text);
    if task.attempts > 1 {
        out.fact("Attempts", &task.attempts.to_string(), text);
    }
    if let Some(error) = &task.error {
        out.fact("Reason", error, Style::default().fg(tone));
    }

    let mut activity = Vec::new();
    let progress = progress_text(task);
    if !progress.is_empty() {
        activity.push(progress);
    }
    for tool in task.recent_tools.iter().rev() {
        if !activity.contains(tool) {
            activity.push(tool.clone());
        }
    }
    if !activity.is_empty() {
        out.heading("Current activity");
        out.boxed(&activity, ACTIVITY_ROWS);
    }
    if let Some(message) = &task.last_message {
        out.heading("Last message");
        out.boxed(std::slice::from_ref(message), MESSAGE_ROWS);
    }
    let next = next_steps(run, task);
    if !next.is_empty() {
        out.heading("Next steps");
        for (i, step) in next.iter().enumerate() {
            let lead = format!("{}. ", i + 1);
            let room = out.width.saturating_sub(lead.len() as u16);
            for (j, row) in ui::wrap(&public_text(step), room.max(1))
                .into_iter()
                .enumerate()
            {
                out.rows.push(Line::from(vec![
                    ui::span(
                        if j == 0 {
                            lead.clone()
                        } else {
                            " ".repeat(lead.len())
                        },
                        th.muted,
                    ),
                    ui::span(row, th.text),
                ]));
            }
        }
    }
    if run.inspecting_node {
        out.heading("Details");
        out.fact("Task ID", &task.id, text);
        out.fact("Role", &task.role, text);
        out.fact("Phase", &task.phase, text);
        out.fact("Policy", &task.policy, text);
        out.fact("Depends on", &task.dependencies.join(", "), text);
        out.fact("Owner", &task.owner, text);
        if task.attempts > 0 {
            out.fact("Attempts", &task.attempts.to_string(), text);
        }
        out.fact("Usage", &task.usage, text);
        if let Some(path) = &task.artifact_file {
            out.fact("Artifact", path, text);
        }
        if let Some(contract) = &task.public_contract {
            out.fact("Contract", contract, text);
        }
    }
}

/// Unfinished downstream agents in the run's own order: the work this agent
/// unblocks. Derived from the graph, never from the model's private plan.
pub fn next_steps(run: &GraphRunSheet, task: &GraphTask) -> Vec<String> {
    let mut reached = std::collections::BTreeSet::from([task.id.as_str()]);
    let mut changed = true;
    while changed {
        changed = false;
        for other in &run.tasks {
            if !reached.contains(other.id.as_str())
                && other
                    .dependencies
                    .iter()
                    .any(|dep| reached.contains(dep.as_str()))
            {
                reached.insert(other.id.as_str());
                changed = true;
            }
        }
    }
    run.tasks
        .iter()
        .filter(|other| other.id != task.id && reached.contains(other.id.as_str()))
        .filter(|other| !matches!(other.state, State::Done | State::Skipped))
        .take(NEXT_STEPS)
        .map(|other| {
            if other.title.trim().is_empty() {
                agent_name(other)
            } else {
                format!("{} ({})", other.title.trim(), other.id)
            }
        })
        .collect()
}

fn run_facts(out: &mut Facts<'_>, run: &GraphRunSheet, inspecting_goal: bool) {
    let th = &out.model.theme;
    let text = Style::default().fg(th.text);
    // Phase and goal already head the command center; the panel only
    // repeats the run when it is in trouble.
    let has_trouble = run.blocked_reason.is_some() || !run.verification.is_empty();
    if inspecting_goal || !has_trouble {
        return;
    }
    out.heading("Run");
    out.fact("Phase", &run.phase, text);
    if let Some(reason) = &run.blocked_reason {
        out.fact("Blocked", reason, Style::default().fg(th.warning));
    }
    for verification in &run.verification {
        out.plain(verification, th.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        ui,
    };

    fn joined(rows: &[Line<'_>]) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn squash(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn details_panel_matches_the_command_center_sections() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 160, 48, false);
        let mut run = fixtures::blueprint_graph();
        run.project = "C:\\Users\\sergi\\Desktop\\davinci".into();
        run.revisions = "0 of 2".into();
        let writer = &mut run.tasks[5];
        writer.title = "Implement remaining work".into();
        writer.branch = Some("feature/superpowers".into());
        writer.model = Some("openai/gpt-6-astra".into());
        writer.tokens = "1.89M in · 11.6K out".into();
        writer.started = "23s ago".into();
        writer.elapsed = "6m18s".into();
        writer.cost = "$1.31".into();
        writer.recent_tools = vec!["edit: src/agents/writer.rs".into()];
        writer.last_message = Some("The baseline release build passed.".into());
        run.tasks[9].title = "Review changes".into();
        model.graph_run = Some(run);
        let drawn = squash(&joined(&inspector_lines(&model, Some("writer"), 52, 100)));
        for fact in [
            "Agent details · Enter to focus",
            "◉ writer",
            "Status: Working",
            "Task: Implement remaining work",
            "Project: C:\\Users\\sergi\\Desktop\\davinci",
            "Branch: feature/superpowers",
            "Model: openai/gpt-6-astra",
            "Tokens: 1.89M in · 11.6K out",
            "Started: 23s ago",
            "Elapsed: 6m18s",
            "Cost: $1.31",
            "Revisions: 0 of 2",
            "Current activity",
            "edit: src/agents/writer.rs",
            "Last message",
            "The baseline release build passed.",
            "Next steps",
            "1. Review changes (review)",
        ] {
            assert!(drawn.contains(fact), "missing {fact}: {drawn}");
        }
    }

    #[test]
    fn missing_live_facts_are_omitted_never_invented() {
        let model = {
            let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 160, 48, false);
            m.graph_run = Some(fixtures::blueprint_graph());
            m
        };
        let drawn = joined(&inspector_lines(&model, Some("review"), 52, 100));
        for absent in ["Model:", "Tokens:", "Started:", "Branch:", "Last message"] {
            assert!(!drawn.contains(absent), "invented {absent}: {drawn}");
        }
        assert!(drawn.contains("Waiting"), "{drawn}");
    }

    #[test]
    fn next_steps_are_unfinished_descendants_in_run_order() {
        let run = fixtures::blueprint_graph();
        let plan = run.tasks.iter().find(|t| t.id == "plan").unwrap();
        let steps = next_steps(&run, plan);
        assert_eq!(
            steps,
            vec![
                "writer",
                "test-analyzer (tests)",
                "reviewer (failure)",
                "verifier (blocked)"
            ]
        );
        let review = run.tasks.iter().find(|t| t.id == "review").unwrap();
        assert!(next_steps(&run, review).is_empty());
    }

    #[test]
    fn selection_shows_activity_without_enter_and_goal_is_readable() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 140, 45, false);
        let mut run = fixtures::blueprint_graph();
        run.inspecting_node = false;
        run.goal = "Original user goal with a final acceptance criterion".into();
        run.tasks[5].recent_tools = vec!["cargo test worker_activity".into()];
        model.graph_run = Some(run);
        let text = joined(&inspector_lines(&model, Some("writer"), 60, 100));
        assert!(text.contains("cargo test worker_activity"), "{text}");
        // The goal heads the command center rather than repeating per agent.
        let frame = joined(&super::super::graph_run::lines_in(&model, 40));
        assert!(frame.contains("Goal:     Original user goal"), "{frame}");
    }

    #[test]
    fn help_panel_lists_every_graph_key() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 140, 45, false);
        model.graph_run = Some(fixtures::blueprint_graph());
        model.graph_canvas.show_help = true;
        let text = joined(&inspector_lines(&model, None, 60, 100));
        for (key, action) in HELP {
            assert!(text.contains(key) && text.contains(action), "{key}: {text}");
        }
    }

    #[test]
    fn goal_panel_remains_bounded_and_pages_to_the_original_prompt_end() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for width in [40, 80, 109, 120, 240] {
            let mut model = Model::new(
                Theme::da_vinci(ColorDepth::TrueColor, true),
                width,
                30,
                false,
            );
            model.screen = crate::davinci::model::Screen::GraphRun;
            let mut run = fixtures::blueprint_graph();
            run.goal = "Original goal acceptance criterion ".repeat(100) + "GOAL_END";
            run.control_status = Some("Retry requested".into());
            model.graph_run = Some(run);
            assert!(super::super::graph_nav::handle_key(
                &mut model,
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)
            ));
            assert!(model.graph_canvas.inspecting_goal);
            let frame = crate::davinci::app::compose_frame(&model, 30);
            assert!(frame.lines.len() <= 30);
            assert!(frame
                .lines
                .iter()
                .all(|row| ui::run_width(&row.spans) <= width));
            let facts = inspector_facts(&model, Some("writer"), width, false);
            assert!(facts.iter().any(|row| row.to_string().contains("GOAL_END")));
            assert!(super::super::graph_nav::handle_key(
                &mut model,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            ));
            assert!(!model.graph_canvas.inspecting_goal);
        }
    }

    #[test]
    fn graph_inspector_public_facts_privacy_and_width() {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            120,
            40,
            false,
        );
        let mut run = fixtures::blueprint_graph();
        let task = &mut run.tasks[5];
        task.owner = "worker-5".into();
        task.attempts = 2;
        task.public_contract = Some("input evidence; output patch".into());
        task.recent_tools = vec![
            "read(src/lib.rs)".into(),
            "thinking: PRIVATE_SENTINEL".into(),
        ];
        task.last_message = Some("reasoning: PRIVATE_MESSAGE".into());
        run.inspecting_node = true;
        model.graph_run = Some(run);
        let rows = inspector_lines(&model, Some("writer"), 40, 100);
        let text = squash(&joined(&rows));
        for fact in [
            "writer",
            "checking public contracts",
            "plan",
            "read-only",
            "worker-5",
            "Attempts: 2",
            "12k",
            "input evidence",
            "read(src/lib.rs)",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        assert!(!text.contains("thinking:") && !text.contains("PRIVATE_SENTINEL"));
        assert!(!text.contains("PRIVATE_MESSAGE"));
        assert!(rows.iter().all(|r| ui::run_width(&r.spans) <= 40));
        for forbidden in [
            "reasoning: PRIVATE_SENTINEL",
            "<thought>PRIVATE_SENTINEL</thought>",
            "<thinking>PRIVATE_SENTINEL",
        ] {
            assert!(!public_text(forbidden).contains("PRIVATE_SENTINEL"));
        }
        assert!(!joined(&inspector_lines(&model, Some("review"), 40, 100)).contains("Owner:"));
    }

    #[test]
    fn graph_inspector_paging_reaches_long_public_contracts() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 80, 32, false);
        let mut run = fixtures::blueprint_graph();
        run.inspecting_node = true;
        run.tasks[5].public_contract = Some("界 artifact ".repeat(100) + "PUBLIC_END");
        model.graph_run = Some(run);
        model.graph_canvas.inspector_scroll = usize::MAX;
        let rows = inspector_lines(&model, Some("writer"), 30, 7);
        assert!(rows.iter().any(|r| r.to_string().contains("PUBLIC_END")));
        assert!(rows.len() <= 7);
    }

    #[test]
    fn graph_inspector_shows_real_status_artifact_and_run_verification() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 80, 40, false);
        let mut run = fixtures::blueprint_graph();
        run.inspecting_node = true;
        run.phase = "blocked".into();
        run.blocked_reason = Some("required tests failed".into());
        run.verification = vec![
            "Verification failed".into(),
            "cargo test · exit 1 · 1.2s".into(),
            "reasoning: PRIVATE_VERIFICATION".into(),
        ];
        run.tasks[5].status = "cancelled".into();
        run.tasks[5].phase = "implement".into();
        run.tasks[5].artifact_file = Some("artifacts/writer.json".into());
        model.graph_run = Some(run);
        let text = squash(&joined(&inspector_lines(&model, Some("writer"), 80, 100)));
        for fact in [
            "Status: Cancelled",
            "Phase: implement",
            "artifacts/writer.json",
            "Phase: blocked",
            "Blocked: required tests failed",
            "Verification failed",
            "cargo test",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        assert!(!text.contains("PRIVATE_VERIFICATION"));
    }
}
