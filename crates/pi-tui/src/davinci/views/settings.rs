//! `3b` — `/settings`. One setting per row, its values as a ramp with the
//! current one marked, and the description of the selected row directly
//! beneath it, so the screen never needs a second panel to explain itself
//! (design.md §1).
//!
//! A setting says which scope it came from — user or project — because a
//! project file silently overriding a user preference is the thing that
//! confuses people.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/settings.ex`.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, SettingRow};
use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, indent, pad, run_width, span, span_on, wrap, MEASURE,
};

/// The label column, as the Elixir reference sets it.
const LABEL: u16 = 24;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    let mut rows = vec![Line::from(vec![
        span("scope ", th.muted),
        Span::styled(" user ", Style::default().fg(th.background).bg(th.primary)),
        span(" ", th.border),
        span(" project ", th.muted),
        span("   tab switches", th.border),
    ])];
    rows.push(blank());

    if model.settings_rows.is_empty() {
        rows.push(Line::from(vec![span(
            "no settings to show — settings.json could not be read",
            th.muted,
        )]));
    } else {
        let selected = model.settings_index % model.settings_rows.len();
        // The sheet is tail-anchored, so a list taller than the body would
        // push its head — and a selection near it — off screen. Show a window
        // around the selection and count what is folded either side.
        const WINDOW: usize = 14;
        let total = model.settings_rows.len();
        let start = selected
            .saturating_sub(WINDOW / 2)
            .min(total.saturating_sub(WINDOW));
        let end = (start + WINDOW).min(total);
        if start > 0 {
            rows.push(Line::from(vec![span(
                format!("… {start} above"),
                th.border,
            )]));
        }
        for (index, setting) in model.settings_rows.iter().enumerate() {
            if index < start || index >= end {
                continue;
            }
            rows.push(setting_row(setting, index == selected, width, th));
            if index == selected {
                for row in wrap(
                    &setting.description,
                    width.saturating_sub(LABEL).saturating_sub(8),
                ) {
                    rows.push(indent(LABEL + 4, vec![span(row, th.muted)]));
                }
            }
        }
        if end < total {
            rows.push(Line::from(vec![span(
                format!("… {} below", total - end),
                th.border,
            )]));
        }
    }

    rows.push(blank());
    rows.push(Line::from(vec![
        span("user ", th.muted),
        span("%USERPROFILE%\\.pi\\agent\\settings.json", th.text),
    ]));
    let project_keys = model.settings_rows.iter().filter(|row| row.project).count();
    rows.push(Line::from(vec![
        span("project ", th.muted),
        span(".pi\\settings.json", th.secondary),
        span(" · ", th.border),
        span("overrides user", th.muted),
        span(" · ", th.border),
        span(
            match project_keys {
                0 => "no keys set".to_string(),
                1 => "1 key set".to_string(),
                n => format!("{n} keys set"),
            },
            th.secondary,
        ),
    ]));
    rows.push(Line::from(vec![span(
        "a flag beats both scopes, for one run",
        th.border,
    )]));
    rows.push(Line::from(vec![
        span("↑↓ setting", th.border),
        span(" · ", th.border),
        span("enter next value", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // A row that would push past the sheet is cut, never wrapped: a long ramp
    // of values on a narrow window loses its tail rather than its shape.
    rows.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, width)))
        .collect()
}

fn setting_row(setting: &SettingRow, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let band = selected.then_some(th.surface);
    let state = if selected {
        State::Active
    } else {
        State::Queued
    };

    let mut spans = vec![
        strong_on(
            format!("{} ", state.glyph()),
            th.state_color(state),
            band,
            th,
        ),
        span_on(
            format!(
                "{:<w$}",
                clip_ellipsis(&setting.label, LABEL),
                w = LABEL as usize
            ),
            if selected { th.text } else { th.muted },
            band,
        ),
    ];
    // The selected value is marked twice — filled, and by position — so the
    // ramp still reads under NO_COLOR (design.md §9).
    for value in &setting.values {
        if *value == setting.value {
            spans.push(Span::styled(
                format!(" {value} "),
                Style::default().fg(th.background).bg(th.primary),
            ));
        } else {
            spans.push(span_on(format!(" {value} "), th.muted, band));
        }
    }
    let scope = if setting.project { "project" } else { "user" };
    let scope_color = if setting.project {
        th.secondary
    } else {
        th.border
    };
    let scope_run = vec![span_on(scope, scope_color, band)];
    let gap = width
        .saturating_sub(run_width(&spans))
        .saturating_sub(run_width(&scope_run))
        .saturating_sub(2)
        .max(1);
    spans.push(pad(gap, band));
    spans.extend(scope_run);
    Line::from(spans)
}

/// A glyph in the theme's emphasis, on the selection band when there is one.
fn strong_on(content: String, color: Color, band: Option<Color>, th: &Theme) -> Span<'static> {
    let mut style = Style::default().fg(color).add_modifier(th.emphasis);
    if let Some(band) = band {
        style = style.bg(band);
    }
    Span::styled(content, style)
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use unicode_width::UnicodeWidthStr;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.settings_rows = vec![
            SettingRow {
                label: "Auto-compact".into(),
                value: "on".into(),
                project: false,
                values: vec!["on".into(), "off".into()],
                description: "Compact the context automatically before it overflows.".into(),
                key: "autoCompact".into(),
            },
            SettingRow {
                label: "Transport".into(),
                value: "websocket-cached".into(),
                project: true,
                values: vec![
                    "sse".into(),
                    "websocket".into(),
                    "websocket-cached".into(),
                    "auto".into(),
                ],
                description: "Preferred transport for providers that support more than one.".into(),
                key: "transport".into(),
            },
        ];
        model.toggle_screen(Screen::Settings);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_setting_shows_its_label_and_the_ramp_of_values() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for setting in &m.settings_rows {
            let row = drawn
                .iter()
                .find(|row| row.contains(&setting.label))
                .unwrap_or_else(|| panic!("{} has no row", setting.label));
            for value in &setting.values {
                assert!(row.contains(value.as_str()), "{value} missing from {row}");
            }
        }
    }

    #[test]
    fn the_selected_row_explains_itself_beneath() {
        let mut m = model(100);
        m.settings_index = 0;
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("Compact the context")),
            "{drawn:?}"
        );
        // Only the selected row carries its description.
        assert!(!drawn.iter().any(|row| row.contains("Preferred transport")));
        m.settings_index = 1;
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("Preferred transport")));
    }

    #[test]
    fn the_selection_is_marked_by_glyph_and_the_current_value_is_filled() {
        let m = model(100);
        let rows = lines(&m);
        let selected = rows
            .iter()
            .find(|row| text(row).contains("Auto-compact"))
            .expect("the selected row");
        assert!(text(selected).starts_with('◉'), "{}", text(selected));
        // The current value is drawn filled: background ink on primary ground.
        let filled = selected
            .spans
            .iter()
            .find(|span| span.content.as_ref() == " on ")
            .expect("the current value chip");
        assert_eq!(filled.style.bg, Some(m.theme.primary));
    }

    #[test]
    fn a_project_scoped_setting_says_so() {
        let m = model(100);
        let rows = lines(&m);
        let row = rows
            .iter()
            .find(|row| text(row).contains("Transport"))
            .expect("the project row");
        assert!(text(&(*row).clone()).contains("project"));
        assert!(row
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.secondary)));
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("1 key set")));
    }

    #[test]
    fn an_empty_sheet_says_why_rather_than_showing_nothing() {
        let mut m = model(100);
        m.settings_rows.clear();
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("no settings to show")));
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let cap = width.min(MEASURE + 14);
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= cap as usize,
                    "row wider than {cap} at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
