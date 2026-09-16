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
    if let Some(group) = &model.graph_canvas.selected_group {
        add(
            "Completed group: ",
            group.strip_prefix("fold:").unwrap_or(group),
        );
        add("", "Enter expands its real members");
    } else if let Some(task) = task {
        add("", &format!("{} {}", task.state.glyph(), task.id));
        add("Work: ", &task.artifact);
        if let Some(error) = &task.error {
            add("Reason: ", error);
        }
        add("Usage: ", &task.usage);
        if run.inspecting_node {
            add("Role: ", &task.role);
            add("Policy: ", &task.policy);
            add("Dependencies: ", &task.dependencies.join(", "));
            add("Owner: ", &task.owner);
            if task.attempts > 0 {
                add("Attempts: ", &task.attempts.to_string());
            }
            for tool in &task.recent_tools {
                add("Recent Tool: ", tool);
            }
            if let Some(contract) = &task.public_contract {
                add("Public Contract: ", contract);
            }
        }
    } else {
        add("", "Arrows select a worker; Enter inspects");
    }
    let count = facts.len();
    let room = max_rows.saturating_sub(2) as usize;
    let offset = model
        .graph_canvas
        .inspector_scroll
        .min(count.saturating_sub(room));
    let mut rows = vec![Line::from(ui::span(
        ui::clip_ellipsis(
            if run.inspecting_node {
                "INSPECTOR · public execution"
            } else {
                "INSPECTOR · Enter for details"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        ui,
    };

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
}
