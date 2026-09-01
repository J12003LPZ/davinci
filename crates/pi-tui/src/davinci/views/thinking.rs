//! `3c` — `/thinking`. Seven levels, each a budget with a meter and a cap,
//! never a bare adjective (design.md §9). The meters are scaled to the 64k
//! ceiling, not to the window; the `max` row says what fraction of the window
//! it would take, because that is the number that hurts.
//!
//! The panel beneath states what a level actually becomes at each provider: a
//! budget in tokens, an effort enum, or nothing at all.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/thinking.ex`. The mockup's
//! "last turn thought 5.1k of 8k" tail is fixture prose with no source in the
//! model, so it is not drawn here.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, ThinkingRow};
use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{
    blank, meter, pad, span, span_strong, truncate_run, wrap, Surface, MEASURE,
};

/// Column widths of the level table, as the Elixir reference sets them.
const LEVEL: usize = 9;
const BUDGET: usize = 7;
/// How many cells each level's meter takes.
const METER_CELLS: u16 = 24;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    let mut rows: Vec<Line<'static>> = Vec::new();
    if model.thinking_rows.is_empty() {
        rows.push(Line::from(vec![span(
            "this model has no thinking knob — the level is kept and ignored",
            th.muted,
        )]));
        return rows;
    }
    let selected = model.thinking_index % model.thinking_rows.len();
    let current = &model.thinking_rows[selected];

    rows.push(Line::from(vec![
        span("thinking ", th.muted),
        span(current.level.clone(), th.primary),
        span(" for this turn and the next", th.muted),
    ]));
    rows.push(Line::from(vec![
        span("every level is a cap, not a promise", th.muted),
        span(" · ", th.border),
        span("the model may think less", th.border),
    ]));
    rows.push(blank());

    rows.push(Line::from(vec![
        pad(2, None),
        span(format!("{:<w$}", "LEVEL", w = LEVEL + 1), th.border),
        span(format!("{:>BUDGET$}  ", "BUDGET"), th.border),
        span(format!("{:<26}", "SHARE OF THE 64k CEILING"), th.border),
        span("WHAT IS SENT", th.border),
    ]));

    for (index, level) in model.thinking_rows.iter().enumerate() {
        rows.push(level_row(level, index == selected, th));
    }

    rows.push(blank());
    let explains: Vec<Vec<Span<'static>>> = [
        "anthropic  sent as a thinking budget in tokens, deducted from the \
         same window as the transcript.",
        "openai  mapped to reasoning effort; seven levels collapse to four, \
         so xhigh and high send the same request.",
        "google  sent as a thinking budget; off means the field is omitted, \
         not zeroed.",
        "a model with no thinking knob keeps the level and ignores it — the \
         status bar says ○ none.",
    ]
    .iter()
    .flat_map(|text| wrap(text, width.saturating_sub(6)))
    .map(|row| vec![span(row, th.muted)])
    .collect();
    rows.extend(
        Surface::new(width, th)
            .title(vec![span("WHAT THE LEVEL DOES", th.primary)])
            .rows(explains)
            .lines(),
    );
    rows.push(blank());
    rows.push(Line::from(vec![
        span("thinking is billed as output", th.muted),
        span(" · ", th.border),
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter set", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));

    rows.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

// No background band on the selected row: `meter` draws on the default
// ground, so a band would tear across the 24 columns of the meter. The glyph
// and the text ramp carry the selection instead (design.md §4).
fn level_row(level: &ThinkingRow, selected: bool, th: &Theme) -> Line<'static> {
    let state = if selected {
        State::Active
    } else if level.warn {
        State::Attention
    } else {
        State::Queued
    };
    // The meter is a magnitude, not a state, so it never takes verdigris.
    let color = if selected {
        th.primary
    } else if level.warn {
        th.warning
    } else {
        th.muted
    };
    let text_color = if selected { th.text } else { th.muted };

    let mut spans = vec![
        span_strong(format!("{} ", state.glyph()), th.state_color(state), th),
        span(format!("{:<w$}", level.level, w = LEVEL + 1), text_color),
        span(format!("{:>BUDGET$}  ", level.budget), text_color),
    ];
    spans.extend(meter(level.fraction, METER_CELLS, th, Some(color)));
    spans.push(span(
        format!("  {}", level.maps_to),
        if level.warn { th.warning } else { th.border },
    ));
    Line::from(spans)
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
        model.thinking_rows = vec![
            ThinkingRow {
                level: "off".into(),
                budget: "0".into(),
                fraction: 0.0,
                maps_to: "disabled → none".into(),
                warn: false,
            },
            ThinkingRow {
                level: "high".into(),
                budget: "16.0k".into(),
                fraction: 0.25,
                maps_to: "16384 → high".into(),
                warn: false,
            },
            ThinkingRow {
                level: "max".into(),
                budget: "64.0k".into(),
                fraction: 1.0,
                maps_to: "! 32% of the window".into(),
                warn: true,
            },
        ];
        model.thinking_index = 1;
        model.toggle_screen(Screen::Thinking);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_level_is_a_budget_with_a_meter_never_a_bare_adjective() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for level in &m.thinking_rows {
            let row = drawn
                .iter()
                .find(|row| row.contains(&format!(" {:<w$}", level.level, w = LEVEL + 1)))
                .unwrap_or_else(|| panic!("{} has no row", level.level));
            assert!(row.contains(&level.budget), "{row}");
            assert!(row.contains('━') || row.contains('─'), "{row}");
            assert!(row.contains(&level.maps_to), "{row}");
        }
    }

    #[test]
    fn the_level_in_hand_is_named_in_the_head_and_marked_by_glyph() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            drawn[0].contains("thinking high for this turn and the next"),
            "{}",
            drawn[0]
        );
        let row = drawn.iter().find(|row| row.starts_with('◉')).unwrap();
        assert!(row.contains("high"), "{row}");
    }

    #[test]
    fn the_greedy_level_warns_in_warning_ink() {
        let m = model(100);
        let rows = lines(&m);
        let row = rows
            .iter()
            .find(|row| text(row).contains("32% of the window"))
            .expect("the max row");
        assert!(text(row).contains('!'), "{}", text(row));
        assert!(row
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.warning)));
    }

    #[test]
    fn the_provider_mapping_panel_names_each_provider() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("WHAT THE LEVEL DOES")));
        for provider in ["anthropic", "openai", "google"] {
            assert!(
                drawn.iter().any(|row| row.contains(provider)),
                "{provider} missing"
            );
        }
    }

    #[test]
    fn a_model_with_no_levels_says_so() {
        let mut m = model(100);
        m.thinking_rows.clear();
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("no thinking knob")));
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
