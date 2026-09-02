//! `/permissions`. The mode in force and every rule, by source.
//!
//! Enter on a mode switches it for the session. Enter on a rule removes it.
//! No TypeScript counterpart.

use ratatui::text::Line;

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{blank, span, span_strong, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut out = vec![
        Line::from(vec![
            span(format!("{} ", glyph::USER), th.primary),
            span("/permissions", th.muted),
        ]),
        blank(),
    ];
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
    out.push(blank());
    out.push(Line::from(vec![span(
        "enter sets a mode or drops a rule · esc closes",
        th.border,
    )]));
    let _ = MEASURE;
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

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
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
        assert!(blob.contains("/permissions"), "{blob}");
        assert!(blob.contains("ask"), "{blob}");
        assert!(blob.contains("bash(git *)"), "{blob}");
    }
}
