//! `5e` — `/workflows`. Deterministic agent workflows view.
//!
//! Renders active and completed workflows with their phases, workers,
//! status glyphs, and elapsed times.

use ratatui::text::Line;

use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{blank, span, span_strong, wrap, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(sheet) = model.workflows.as_ref() else {
        return vec![Line::from(vec![span(
            "no workflows tracked — /workflow <goal> starts one",
            th.muted,
        )])];
    };

    let mut out: Vec<Line<'static>> = Vec::new();

    if sheet.workflows.is_empty() {
        out.push(Line::from(vec![span(
            "no active workflows — /workflow <goal> starts one",
            th.muted,
        )]));
    } else {
        let selected = if !sheet.workflows.is_empty() {
            model.workflow_index % sheet.workflows.len()
        } else {
            0
        };

        for (index, wf) in sheet.workflows.iter().enumerate() {
            out.push(workflow_row(wf, index == selected, th));
            if let Some(err) = &wf.error {
                for line in wrap(err, MEASURE.saturating_sub(6)) {
                    out.push(Line::from(vec![
                        span("      ", th.muted),
                        span(line, th.error),
                    ]));
                }
            }
        }
    }

    out.push(blank());
    let running = sheet
        .workflows
        .iter()
        .filter(|w| w.status == "running")
        .count();
    let completed = sheet
        .workflows
        .iter()
        .filter(|w| w.status == "completed")
        .count();
    out.push(Line::from(vec![span(
        format!(
            "{running} running · {completed} completed · {} total",
            sheet.workflows.len()
        ),
        th.muted,
    )]));

    out
}

fn workflow_row(
    wf: &crate::davinci::model::WorkflowRow,
    selected: bool,
    th: &Theme,
) -> Line<'static> {
    let (state, color) = match wf.status.as_str() {
        "completed" => (State::Done, th.success),
        "running" => (State::Active, th.primary),
        "paused" => (State::Queued, th.muted),
        "cancelled" => (State::Skipped, th.border),
        _ => (State::Failed, th.error),
    };

    let selector = if selected {
        span(format!("{} ", glyph::PROMPT), th.primary)
    } else {
        span("  ", th.border)
    };

    let mut spans = vec![
        selector,
        span_strong(format!("{} ", state.glyph()), color, th),
        span(
            format!("{:<16}", wf.id.chars().take(8).collect::<String>()),
            th.muted,
        ),
        span(format!("{:<20}", wf.name), th.text),
        span(format!("{:<12}", wf.status), color),
    ];

    if !wf.phases.is_empty() {
        let mut phase_spans = Vec::new();
        for (idx, (p_name, _)) in wf.phases.iter().enumerate() {
            if idx > 0 {
                phase_spans.push(" ── ");
            }
            phase_spans.push(p_name.as_str());
        }
        spans.push(span(phase_spans.concat(), th.muted));
    }

    Line::from(spans)
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let sheet = model.workflows.as_ref();
    let total = sheet.map(|s| s.workflows.len()).unwrap_or(0);
    let running = sheet
        .map(|s| s.workflows.iter().filter(|w| w.status == "running").count())
        .unwrap_or(0);

    SheetChrome {
        header_right: facts(
            th,
            vec![
                vec![span(format!("{total} workflows"), th.text)],
                vec![span(format!("{running} running"), th.muted)],
            ],
        ),
        status_third: Some(vec![span("/workflow <goal>", th.muted)]),
        status_right: None,
        hints: vec![hint(
            th,
            "enter inspects · /workflow-stop <id> stops · esc close",
        )],
        escape: Some("esc close"),
        composer: Composer::Prompt("/workflows"),
        echo: Some("/workflows".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Model, WorkflowRow, WorkflowsSheet};
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn test_workflows_view_empty() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let model = Model::new(theme, 80, 24, false);
        let rendered = lines(&model);
        assert!(!rendered.is_empty());
        assert!(rendered[0].to_string().contains("no workflows tracked"));
    }

    #[test]
    fn test_workflows_view_with_data() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let mut model = Model::new(theme, 80, 24, false);
        model.workflows = Some(WorkflowsSheet {
            workflows: vec![WorkflowRow {
                id: "018f-1234-5678-9abc".into(),
                name: "code-audit".into(),
                status: "running".into(),
                phases: vec![
                    ("investigate".into(), "completed".into()),
                    ("plan".into(), "running".into()),
                ],
                started_ms: 1000,
                elapsed: "5s".into(),
                error: None,
            }],
            selected_index: 0,
        });

        let rendered = lines(&model);
        assert!(!rendered.is_empty());
        let text = rendered
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("code-audit"));
        assert!(text.contains("running"));
        assert!(text.contains("1 running"));
    }
}
