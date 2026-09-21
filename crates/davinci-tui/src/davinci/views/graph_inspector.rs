//! Only public, explicitly exposed execution facts enter the inspector.
use crate::davinci::model::Model;
use crate::davinci::{theme::State, ui};
use ratatui::text::Line;

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

pub fn inspector_lines(
    model: &Model,
    selected: Option<&str>,
    width: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    if max_rows == 0 {
        return Vec::new();
    }
    let Some(run) = &model.graph_run else {
        return Vec::new();
    };
    let facts = inspector_facts(model, selected, width);
    let count = facts.len();
    let room = max_rows.saturating_sub(2) as usize;
    let offset = model
        .graph_canvas
        .inspector_scroll
        .min(count.saturating_sub(room));
    let mut rows = vec![Line::from(ui::span(
        ui::clip_ellipsis(
            if model.graph_canvas.inspecting_goal {
                "Goal · g worker activity"
            } else if run.inspecting_node {
                "Agent details"
            } else {
                "Activity · Enter details"
            },
            width,
        ),
        model.theme.primary,
    ))];
    rows.extend(facts.into_iter().skip(offset).take(room));
    if max_rows > 1 {
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
    let maximum = inspector_facts(model, selected, width)
        .len()
        .saturating_sub(height.saturating_sub(2) as usize);
    model.graph_canvas.inspector_scroll = model
        .graph_canvas
        .inspector_scroll
        .min(maximum)
        .saturating_add_signed(delta)
        .min(maximum);
}

fn inspector_facts(model: &Model, selected: Option<&str>, width: u16) -> Vec<Line<'static>> {
    let Some(run) = &model.graph_run else {
        return Vec::new();
    };
    let task = selected
        .and_then(|id| run.tasks.iter().find(|t| t.id == id))
        .or_else(|| run.tasks.iter().find(|t| t.state == State::Active));
    let mut facts = Vec::new();
    let mut add = |label: &str, value: &str| {
        if !value.is_empty() {
            facts.extend(
                ui::wrap(&format!("{label}{}", public_text(value)), width)
                    .into_iter()
                    .map(|s| Line::from(ui::span(s, model.theme.text))),
            );
        }
    };
    if model.graph_canvas.inspecting_goal {
        add("Status: ", &run.lifecycle);
        add("Phase: ", &run.phase);
        add("Original prompt: ", &run.goal);
        add("Progress", " ");
        for task in &run.tasks {
            add(
                "",
                &format!("{} {} · {}", task.state.glyph(), task.id, task.status),
            );
        }
    } else if let Some(group) = &model.graph_canvas.selected_group {
        add(
            "Completed group: ",
            group.strip_prefix("fold:").unwrap_or(group),
        );
        add("", "Enter expands its real members");
    } else if let Some(task) = task {
        add(
            "",
            &format!("{} {}", task.state.glyph(), task.display_title()),
        );
        if let Some(branch) = &task.branch {
            add("Branch: ", branch);
        }
        if let Some(worktree) = &task.worktree {
            add("Worktree: ", worktree);
        }
        add("Status: ", &task.status);
        add("Phase: ", &task.phase);
        add("Work: ", &task.artifact);
        for activity in task.recent_tools.iter().rev() {
            add("Activity: ", activity);
        }
        if let Some(error) = &task.error {
            add("Reason: ", error);
        }
        add("Usage: ", &task.usage);
        if let Some(path) = &task.artifact_file {
            add("Artifact: ", path);
        }
        if run.inspecting_node {
            add("Task ID: ", &task.id);
            add("Role: ", &task.role);
            add("Policy: ", &task.policy);
            add("Dependencies: ", &task.dependencies.join(", "));
            add("Owner: ", &task.owner);
            if task.attempts > 0 {
                add("Attempts: ", &task.attempts.to_string());
            }
            if let Some(contract) = &task.public_contract {
                add("Public Contract: ", contract);
            }
        }
    } else {
        add("", "Arrows select a worker; Enter inspects");
    }
    add("Run phase: ", &run.phase);
    if !model.graph_canvas.inspecting_goal {
        add("Main goal (g): ", &run.goal);
    }
    if let Some(reason) = &run.blocked_reason {
        add("Run blocked: ", reason);
    }
    for verification in &run.verification {
        add("", verification);
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        ui,
    };

    #[test]
    fn selection_shows_activity_without_enter_and_goal_is_readable() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 140, 45, false);
        let mut run = fixtures::blueprint_graph();
        run.inspecting_node = false;
        run.goal = "Original user goal with a final acceptance criterion".into();
        run.tasks[5].recent_tools = vec!["cargo test worker_activity".into()];
        model.graph_run = Some(run);
        let text = inspector_lines(&model, Some("writer"), 60, 100)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("cargo test worker_activity"), "{text}");
        assert!(text.contains("Original user goal"), "{text}");
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
            let facts = inspector_facts(&model, Some("writer"), width);
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
        run.inspecting_node = true;
        model.graph_run = Some(run);
        let rows = inspector_lines(&model, Some("writer"), 40, 100);
        let text = rows
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
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
        assert!(rows.iter().all(|r| ui::run_width(&r.spans) <= 40));
        for forbidden in [
            "reasoning: PRIVATE_SENTINEL",
            "<thought>PRIVATE_SENTINEL</thought>",
            "<thinking>PRIVATE_SENTINEL",
        ] {
            assert!(!public_text(forbidden).contains("PRIVATE_SENTINEL"));
        }
        assert!(!inspector_lines(&model, Some("review"), 40, 100)
            .iter()
            .any(|r| r.to_string().contains("Owner:")));
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
        let text = inspector_lines(&model, Some("writer"), 80, 100)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for fact in [
            "Status: cancelled",
            "Phase: implement",
            "artifacts/writer.json",
            "Run phase: blocked",
            "required tests failed",
            "Verification failed",
            "cargo test",
        ] {
            assert!(text.contains(fact), "missing {fact}: {text}");
        }
        assert!(!text.contains("PRIVATE_VERIFICATION"));
    }
}
