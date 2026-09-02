//! `3c` — `/thinking`. Seven levels, each a budget with a meter and a cap,
//! never a bare adjective (design.md §9). The meters are scaled to the 64k
//! ceiling, not to the window; the `max` row says what fraction of the window
//! it would take, because that is the number that hurts.
//!
//! The panel beneath states what a level actually becomes at each provider: a
//! budget in tokens, an effort enum, or nothing at all.
//!
//! Mirrors artboard `3c` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::{Model, ThinkingRow};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, column_header, footnote, meter, selection_bar, span, span_on, span_strong, truncate_run,
    wrap, Surface,
};

/// Column widths of the level table, as the artboard sets them.
const LEVEL: u16 = 9;
const BUDGET: u16 = 7;
/// How many cells each level's meter takes.
const METER_CELLS: u16 = 24;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

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

    let sent = if model.model_name.is_empty() {
        "WHAT IS SENT".to_string()
    } else {
        format!("{} → GPT", model.model_name.to_uppercase())
    };
    rows.extend(column_header(
        width,
        &[
            ("", LEAD - 1, false),
            ("LEVEL", LEVEL, false),
            ("BUDGET", BUDGET, true),
            ("SHARE OF THE 64k CEILING", METER_CELLS + 1, false),
            (sent.as_str(), 0, false),
        ],
        th,
    ));

    for (index, level) in model.thinking_rows.iter().enumerate() {
        rows.push(level_row(level, index == selected, th));
    }

    rows.push(blank());
    let explains: Vec<Vec<Span<'static>>> = [
        (
            "anthropic",
            "sent as a thinking budget in tokens, deducted from the same window \
             as the transcript.",
        ),
        (
            "openai",
            "mapped to reasoning effort; seven levels collapse to four, so xhigh \
             and high send the same request.",
        ),
        (
            "google",
            "sent as a thinking budget; off means the field is omitted, not \
             zeroed.",
        ),
    ]
    .iter()
    .flat_map(|(provider, text)| {
        let key = format!("{provider:<10}│ ");
        let lead = key.chars().count() as u16;
        wrap(text, width.saturating_sub(6 + lead))
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                if index == 0 {
                    vec![
                        span(format!("{provider:<10}"), th.secondary),
                        span("│ ", th.border),
                        span(row, th.muted),
                    ]
                } else {
                    vec![
                        span(" ".repeat(lead as usize), th.border),
                        span(row, th.muted),
                    ]
                }
            })
            .collect::<Vec<_>>()
    })
    .chain(
        wrap(
            "a model with no thinking knob keeps the level and ignores it — the \
             status bar says ○ none.",
            width.saturating_sub(6),
        )
        .into_iter()
        .map(|row| vec![span(row, th.border)]),
    )
    .collect();
    rows.extend(
        Surface::new(width, th)
            .title(vec![span("WHAT THE LEVEL DOES", th.primary)])
            .rows(explains)
            .lines(),
    );
    rows.push(blank());

    let mut last_turn = Vec::new();
    if !model.facts.thinking_last_turn.is_empty() {
        last_turn.push(span("last turn thought ", th.muted));
        last_turn.push(span(model.facts.thinking_last_turn.clone(), th.text));
        last_turn.push(span(" · ", th.border));
        last_turn.push(span(glyph::DONE, th.success));
        last_turn.push(span(" under budget", th.muted));
    }
    let mut billed = vec![span("thinking is billed as output", th.border)];
    if model.facts.thinking_output_share > 0.0 {
        let share = model.facts.thinking_output_share;
        billed.push(span(" · ", th.border));
        billed.push(span(th.pie(share), th.primary));
        billed.push(span(
            format!(
                " {}% of this session's output tokens",
                (share * 100.0) as u32
            ),
            th.border,
        ));
    }
    if last_turn.is_empty() {
        rows.push(Line::from(billed));
    } else {
        rows.extend(footnote(width, last_turn, billed, th));
    }

    rows.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the knob and the reserve in the
/// header, the level in hand in the status bar, the command in the composer.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let level = model
        .thinking_rows
        .get(model.thinking_index % model.thinking_rows.len().max(1))
        .map(|row| row.level.clone())
        .unwrap_or_default();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                if model.model_name.is_empty() {
                    Vec::new()
                } else {
                    vec![span(model.model_name.clone(), th.muted)]
                },
                if model.thinking_rows.is_empty() {
                    Vec::new()
                } else {
                    vec![span("budget knob", th.muted)]
                },
                if model.facts.thinking_reserve.is_empty() {
                    Vec::new()
                } else {
                    vec![span(model.facts.thinking_reserve.clone(), th.muted)]
                },
            ],
        ),
        status_third: (!level.is_empty())
            .then(|| vec![span("thinking ", th.muted), span(level, th.primary)]),
        status_right: None,
        hints: vec![
            hint(th, "↑↓ move"),
            hint(th, "enter select"),
            hint(th, "shift+tab cycle"),
            hint_dim(th, "ctrl+t toggle off"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("/thinking high"),
        echo: None,
    }
}

// The selection band stops at the meter: `meter` draws on the default
// ground, so a band would tear across the 24 columns of the meter. The bar,
// the glyph and the text ramp carry the selection instead (design.md §4).
fn level_row(level: &ThinkingRow, selected: bool, th: &Theme) -> Line<'static> {
    let band = selected.then_some(th.surface);
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
        selection_bar(selected, th),
        {
            let mut glyph = span_strong(format!("{} ", state.glyph()), th.state_color(state), th);
            if let Some(band) = band {
                glyph.style = glyph.style.bg(band);
            }
            glyph
        },
        span_on(
            format!("{:<w$} ", level.level, w = LEVEL as usize),
            text_color,
            band,
        ),
        span_on(
            format!("{:>w$} ", level.budget, w = BUDGET as usize),
            text_color,
            band,
        ),
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
                .find(|row| row.contains(&format!(" {:<w$} ", level.level, w = LEVEL as usize)))
                .unwrap_or_else(|| panic!("{} has no row", level.level));
            assert!(row.contains(&level.budget), "{row}");
            assert!(row.contains('━') || row.contains('─'), "{row}");
            assert!(row.contains(&level.maps_to), "{row}");
        }
    }

    #[test]
    fn the_level_in_hand_is_named_in_the_head_and_marked_by_bar_and_glyph() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            drawn[0].contains("thinking high for this turn and the next"),
            "{}",
            drawn[0]
        );
        let row = drawn.iter().find(|row| row.starts_with("▌  ◉")).unwrap();
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
    fn the_provider_mapping_panel_keys_each_provider() {
        let m = model(100);
        let rows = lines(&m);
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("WHAT THE LEVEL DOES")));
        for provider in ["anthropic", "openai", "google"] {
            let row = rows
                .iter()
                .find(|row| text(row).contains(&format!("{provider:<10}│")))
                .unwrap_or_else(|| panic!("{provider} missing"));
            assert!(row
                .spans
                .iter()
                .any(|span| span.content.as_ref().trim() == provider
                    && span.style.fg == Some(m.theme.secondary)));
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
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3c");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "sonnet │ budget knob │ reserve 10k");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "thinking medium");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Prompt("/thinking high"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("↑↓ move │ enter select │ shift+tab cycle │ ctrl+t toggle off"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("SONNET → GPT")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("last turn thought 5.1k of 8k · ✓ under budget")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("38% of this session's output tokens")));
        assert!(!drawn.iter().any(|row| row.contains("esc close")));
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
