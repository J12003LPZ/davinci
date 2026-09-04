//! `/permissions`. The mode in force and every rule, by source.
//!
//! Enter on a mode switches it for the session. Enter on a rule removes it.
//! No TypeScript counterpart.

use ratatui::text::Line;

use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{span, span_strong};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut out: Vec<Line<'static>> = Vec::new();
    if model.permission_rows.is_empty() {
        out.push(Line::from(vec![span(
            "no permission policy loaded",
            th.muted,
        )]));
        return out;
    }
    let selected = model.permission_index % model.permission_rows.len();
    for (index, row) in model.permission_rows.iter().enumerate() {
        out.push(permission_row(row, index == selected, th));
    }
    out
}

fn permission_row(
    row: &crate::davinci::model::PermissionRow,
    selected: bool,
    th: &Theme,
) -> Line<'static> {
    let mark = if row.current {
        State::Done.glyph()
    } else if selected {
        State::Active.glyph()
    } else {
        State::Queued.glyph()
    };
    let color = if row.current {
        th.success
    } else if selected {
        th.primary
    } else {
        th.text
    };
    Line::from(vec![
        span(format!("{} ", glyph::BRANCH), th.border),
        span_strong(format!("{mark} "), color, th),
        span(format!("{:<22}", row.label), color),
        span(row.detail.clone(), th.muted),
    ])
}

/// The sheet's frame (design.md §11): the mode in force and the rule count
/// in the header, the mode in the status bar, the one key on the hint row.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let mode = model
        .permission_rows
        .iter()
        .find(|row| row.kind == "mode" && row.current)
        .map(|row| row.label.clone());
    let rules = model
        .permission_rows
        .iter()
        .filter(|row| row.kind == "rule")
        .count();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                mode.clone()
                    .map(|mode| vec![span("mode ", th.muted), span(mode, th.text)])
                    .unwrap_or_default(),
                if model.permission_rows.is_empty() {
                    Vec::new()
                } else {
                    vec![span(
                        format!("{rules} rule{}", if rules == 1 { "" } else { "s" }),
                        th.muted,
                    )]
                },
            ],
        ),
        status_third: mode.map(|mode| vec![span(mode, th.muted)]),
        status_right: None,
        hints: vec![hint(th, "enter sets a mode or drops a rule")],
        escape: Some("esc close"),
        composer: Composer::Prompt("/permissions"),
        echo: Some("/permissions".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Model, PermissionRow};
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn the_sheet_marks_the_mode_in_force() {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            100,
            24,
            false,
        );
        model.permission_rows = vec![
            PermissionRow {
                label: "ask".into(),
                detail: "read tools run; edits ask".into(),
                current: true,
                kind: "mode".into(),
                key: "ask".into(),
                source: String::new(),
            },
            PermissionRow {
                label: "bash(git *)".into(),
                detail: "session".into(),
                current: false,
                kind: "rule".into(),
                key: "bash(git *)".into(),
                source: "session".into(),
            },
        ];
        let blob: String = lines(&model)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!blob.contains("/permissions"), "{blob}");
        assert!(!blob.contains("esc closes"), "{blob}");
        assert!(blob.contains("ask"), "{blob}");
        assert!(blob.contains("bash(git *)"), "{blob}");
        let c = chrome(&model);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "mode ask │ 1 rule");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "ask");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Prompt("/permissions"));
        assert_eq!(c.echo.as_deref(), Some("/permissions"));
    }
}
