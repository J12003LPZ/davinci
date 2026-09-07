//! Tracked workflows with full identifiers, phase states, elapsed time and
//! errors. This is a status view, not a picker with an unimplemented Enter.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::ui::{section_detail, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

fn state(status: &str) -> State {
    match status {
        "completed" => State::Done,
        "running" => State::Active,
        "paused" | "pending" => State::Queued,
        "cancelled" | "skipped" => State::Skipped,
        _ => State::Failed,
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .workflows
        .as_ref()
        .filter(|sheet| !sheet.workflows.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No workflows tracked. Use /workflow <goal> from the conversation to start one.",
        );
    };
    let mut rows = Vec::new();
    for workflow in &sheet.workflows {
        rows.extend(section_state(
            width,
            th,
            state(&workflow.status),
            &workflow.name,
        ));
        rows.extend(section_detail(width, th, &format!("ID: {}", workflow.id)));
        rows.extend(section_detail(
            width,
            th,
            &format!("Status: {}", workflow.status),
        ));
        if !workflow.elapsed.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                &format!("Elapsed: {}", workflow.elapsed),
            ));
        }
        for (phase, status) in &workflow.phases {
            rows.extend(section_state(
                width,
                th,
                state(status),
                &format!("  {phase} · {status}"),
            ));
        }
        if let Some(error) = &workflow.error {
            let mut error = section_detail(width, th, error);
            for row in &mut error {
                for span in &mut row.spans {
                    span.style.fg = Some(th.error);
                }
            }
            rows.extend(error);
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let total = model
        .workflows
        .as_ref()
        .map_or(0, |sheet| sheet.workflows.len());
    let running = model.workflows.as_ref().map_or(0, |sheet| {
        sheet
            .workflows
            .iter()
            .filter(|workflow| workflow.status == "running")
            .count()
    });
    SheetChrome {
        header_right: vec![span(format!("{total} workflows"), th.muted)],
        status_third: Some(vec![span(format!("{running} running"), th.muted)]),
        hints: vec![hint(th, "↑↓ scroll")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{WorkflowRow, WorkflowsSheet},
        theme::{ColorDepth, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.workflows = Some(WorkflowsSheet {
            workflows: vec![WorkflowRow {
                id: "workflow-full-identifier-12345".into(),
                name: "Review changes".into(),
                status: "paused".into(),
                phases: vec![
                    ("analyze".into(), "completed".into()),
                    ("review".into(), "paused".into()),
                ],
                started_ms: 0,
                elapsed: "3m 12s".into(),
                error: Some("Worker unavailable".into()),
            }],
            selected_index: 0,
        });
        m
    }
    #[test]
    fn workflow_status_keeps_full_identifiers_phase_states_and_errors_without_fake_focus() {
        let m = model(80);
        let rows = lines(&m);
        let drawn = rows
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for value in [
            "workflow-full-identifier-12345",
            "Review changes",
            "analyze · completed",
            "review · paused",
            "3m 12s",
            "Worker unavailable",
        ] {
            assert!(drawn.contains(value));
        }
        assert!(ui::focused_row(&rows).is_none());
    }
    #[test]
    fn empty_and_narrow_workflows_are_bounded() {
        let mut m = model(80);
        m.workflows = None;
        assert!(lines(&m)[0].to_string().contains("No workflows"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
