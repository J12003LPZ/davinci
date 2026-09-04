//! `3b` — `/settings`. One setting per row, its values as a ramp with the
//! current one marked, and the description of the selected row directly
//! beneath it, so the screen never needs a second panel to explain itself
//! (design.md §1).
//!
//! A setting says which scope it came from — user or project — because a
//! project file silently overriding a user preference is the thing that
//! confuses people.
//!
//! Mirrors artboard `3b` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::{Model, SettingRow};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, footnote, indent, pad, run_width, selection_bar, span, span_on,
    truncate_run, wrap,
};

/// The label column, as the artboard sets it.
const LABEL: u16 = 24;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

    let mut rows: Vec<Line<'static>> = Vec::new();
    if model.settings_rows.is_empty() {
        rows.push(Line::from(vec![span(
            "no settings to show — settings.json could not be read",
            th.muted,
        )]));
    } else {
        let selected = model.settings_index % model.settings_rows.len();
        for (index, setting) in model.settings_rows.iter().enumerate() {
            rows.push(setting_row(setting, index == selected, width, th));
            if index == selected {
                // The description sits under the ramp, in the value column.
                for row in wrap(
                    &setting.description,
                    width.saturating_sub(LEAD + LABEL + 12),
                ) {
                    rows.push(indent(LEAD + LABEL + 1, vec![span(row, th.muted)]));
                }
            }
        }
    }

    rows.push(blank());
    rows.extend(footnote(
        width,
        vec![
            span("user ", th.muted),
            span("%USERPROFILE%\\.pi\\agent\\settings.json", th.text),
        ],
        vec![
            span("written on change", th.border),
            span(" · ", th.border),
            span("no restart", th.border),
        ],
        th,
    ));
    let project_keys = model.settings_rows.iter().filter(|row| row.project).count();
    rows.extend(footnote(
        width,
        vec![
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
        ],
        vec![span("a flag beats both scopes, for one run", th.border)],
        th,
    ));
    // A row that would push past the sheet is cut, never wrapped: a long ramp
    // of values on a narrow window loses its tail rather than its shape.
    rows.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the scope tabs in the header, the key
/// count in the status bar, no composer.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let keys = if model.facts.settings_keys > 0 {
        model.facts.settings_keys
    } else {
        model.settings_rows.len()
    };
    SheetChrome {
        header_right: facts(
            th,
            vec![
                vec![span("scope ", th.muted), span("user", th.text)],
                vec![span("project", th.muted)],
                vec![span("tab switches", th.border)],
            ],
        ),
        status_third: (keys > 0).then(|| vec![span(format!("{keys} keys"), th.muted)]),
        status_right: None,
        hints: vec![
            hint(th, "↑↓ setting"),
            hint(th, "enter next value"),
            hint_dim(th, "tab scope"),
            hint_dim(th, "r reset to default"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
}

fn setting_row(setting: &SettingRow, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let band = selected.then_some(th.surface);
    let state = if selected {
        State::Active
    } else {
        State::Queued
    };
    let boolean = setting.values.len() == 2
        && setting.values.iter().any(|v| v == "on")
        && setting.values.iter().any(|v| v == "off");

    let mut spans = vec![
        selection_bar(selected, th),
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
    // ramp still reads under NO_COLOR (design.md §9). A switch says `✓ on`,
    // as the artboard writes it.
    for value in &setting.values {
        if *value == setting.value {
            if boolean && value == "on" {
                spans.push(span_on(format!(" {} on ", glyph::DONE), th.success, band));
            } else {
                spans.push(Span::styled(
                    format!(" {value} "),
                    Style::default().fg(th.background).bg(th.primary),
                ));
            }
        } else {
            spans.push(span_on(format!(" {value} "), th.muted, band));
        }
    }
    if !setting.note.is_empty() {
        spans.push(span_on(format!("  {}", setting.note), th.border, band));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
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
                note: String::new(),
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
                note: String::new(),
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
    fn the_selection_is_marked_by_bar_and_glyph_and_a_switch_reads_on() {
        let m = model(100);
        let rows = lines(&m);
        let selected = rows
            .iter()
            .find(|row| text(row).contains("Auto-compact"))
            .expect("the selected row");
        assert!(text(selected).starts_with("▌  ◉"), "{}", text(selected));
        assert!(text(selected).contains("✓ on"), "{}", text(selected));
        let on = selected
            .spans
            .iter()
            .find(|span| span.content.as_ref().contains("✓ on"))
            .expect("the switch");
        assert_eq!(on.style.fg, Some(m.theme.success));
        assert_eq!(on.style.bg, Some(m.theme.surface), "the row is tinted");
    }

    #[test]
    fn a_ramp_value_in_hand_is_a_filled_chip() {
        let mut m = model(100);
        m.settings_index = 1;
        let rows = lines(&m);
        let row = rows
            .iter()
            .find(|row| text(row).contains("Transport"))
            .expect("the transport row");
        let filled = row
            .spans
            .iter()
            .find(|span| span.content.as_ref() == " websocket-cached ")
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
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3b");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "scope user │ project │ tab switches");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "24 keys");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(hint.starts_with("↑↓ setting │ enter next value"), "{hint}");
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
        // The body carries no hint line of its own and no scope row.
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("esc close")));
        assert!(!drawn[0].contains("scope"), "{}", drawn[0]);
        assert!(drawn
            .iter()
            .any(|row| row.contains("written on change · no restart")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("registers /skill:name")));
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= width as usize,
                    "row wider than {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
