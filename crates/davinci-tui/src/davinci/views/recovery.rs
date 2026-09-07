//! Recovery: the failure, retained state, and runtime-supplied aftermath.
//! Do not infer an interruption, successful persistence, or a scheduled retry
//! merely from the existence of a failed-run record.

use super::sheet::{Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(run) = &model.failed_run else {
        return section_detail(width, th, "No interrupted or failed turn to review.");
    };
    let headline = if run.error.is_empty() {
        "The turn did not complete"
    } else {
        &run.error
    };
    let mut rows = section_detail(width, th, &format!("! {headline}"));
    for row in &mut rows {
        for s in &mut row.spans {
            s.style.fg = Some(th.error);
        }
    }
    for (label, value) in [
        ("Request", &run.prompt),
        ("Retained", &run.kept),
        ("Files written", &run.files_written),
        ("Usage", &run.billed),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    if !run.tools.is_empty() {
        rows.extend(section_detail(width, th, "Tool activity"));
    }
    for (state, text, detail) in &run.tools {
        let label = if detail.is_empty() {
            format!("{} {text}", state.glyph())
        } else {
            format!("{} {text} · {detail}", state.glyph())
        };
        let mut tool_rows = section_detail(width, th, &label);
        if *state == State::Failed {
            for row in &mut tool_rows {
                for s in &mut row.spans {
                    s.style.fg = Some(th.error);
                }
            }
        }
        rows.extend(tool_rows);
    }
    if !run.retry.is_empty() {
        rows.extend(section_detail(width, th, &run.retry));
    }
    for (state, text) in &run.aftermath {
        let prefix = if *state == State::Attention { "! " } else { "" };
        rows.extend(section_detail(width, th, &format!("{prefix}{text}")));
    }
    rows.extend(section_detail(
        width,
        th,
        "Close this view, then send a new instruction to continue or change direction.",
    ));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    SheetChrome {
        status_third: Some(vec![span("turn stopped", model.theme.warning)]),
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::FailedRun,
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
        m.failed_run = Some(FailedRun {
            error: "request failed".into(),
            kept: "12 tokens".into(),
            files_written: "2".into(),
            tools: vec![(State::Failed, "read sample.rs".into(), "not found".into())],
            ..Default::default()
        });
        m
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn failures_show_actual_details_not_invented_user_or_persistence_events() {
        let drawn = text(&model(80));
        for value in [
            "request failed",
            "12 tokens",
            "Files written: 2",
            "read sample.rs",
            "not found",
            "Close this view",
        ] {
            assert!(drawn.contains(value));
        }
        for value in [
            "You stopped",
            "session written",
            "ctrl+c",
            "retry now",
            "finish on opus",
        ] {
            assert!(!drawn.contains(value));
        }
    }
    #[test]
    fn retry_and_aftermath_are_only_present_when_supplied() {
        let mut m = model(80);
        assert!(!text(&m).contains("retrying"));
        m.failed_run.as_mut().unwrap().retry = "retrying in 9s".into();
        m.failed_run.as_mut().unwrap().aftermath =
            vec![(State::Attention, "follow-ups cleared".into())];
        assert!(text(&m).contains("retrying in 9s") && text(&m).contains("follow-ups cleared"));
    }
    #[test]
    fn empty_and_narrow_failures_do_not_overflow() {
        let mut m = model(80);
        m.failed_run = None;
        assert!(text(&m).contains("No interrupted or failed turn"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
