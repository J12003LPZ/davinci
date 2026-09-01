//! `1c` — the plan sheet.
//!
//! Roman numerals in a 4-column gutter, a footer that reads `constructio
//! III / V` with a tick meter, and one decorative compass in the top-right
//! that is clipped by its own layer, so the panel label is never cut
//! (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/disegno.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{pad, run_width, span, span_strong, surface_rule, Surface, MEASURE};

/// The compass, one row per plan step, drawn in the right margin.
const COMPASS: [&str; 5] = [
    "   ·─────·",
    " ╭─╲  │  ╱─╮",
    "─┼───┼───┼─",
    " ╰─╱  │  ╲─╯",
    "   ·─────·",
];

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 12);
    let inner = width.saturating_sub(4);
    // `constructio III / V` counts the step in hand, not the steps behind
    // (`1c`); with nothing active it falls back to what is done.
    let current = model
        .plan
        .iter()
        .position(|step| step.state == State::Active)
        .map(|index| index + 1)
        .unwrap_or_else(|| {
            model
                .plan
                .iter()
                .filter(|step| step.state == State::Done)
                .count()
        });
    let total = model.plan.len();

    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    for (index, step) in model.plan.iter().enumerate() {
        let verb_color = if step.state == State::Queued {
            th.muted
        } else {
            th.text
        };
        let mut left = vec![
            span(format!("{:>3} ", step.numeral), th.border),
            span_strong(
                format!("{} ", step.state.glyph()),
                th.state_color(step.state),
                th,
            ),
            span(step.verb.clone(), verb_color),
        ];
        if let Some(target) = &step.target {
            left.push(span(" · ", th.border));
            left.push(span(target.clone(), th.secondary));
        }

        // The compass is decoration; it is dropped below 100 columns (§7) and
        // it lives in its own right-hand run so it can never reach the label.
        let right = if model.decoration() {
            vec![span(
                COMPASS.get(index).copied().unwrap_or("").to_string(),
                th.border,
            )]
        } else {
            Vec::new()
        };

        let gap = inner
            .saturating_sub(run_width(&left))
            .saturating_sub(run_width(&right))
            .max(1);
        let mut row = left;
        row.push(pad(gap, None));
        row.extend(right);
        body.push(row);
    }

    if body.is_empty() {
        // A plan sheet with nothing on it says so rather than drawing an
        // empty frame that looks like a failure — and there is no progress to
        // meter, so the tally goes with it rather than reading `/`.
        body.push(vec![span("no plan drawn for this project yet", th.muted)]);
        body.push(surface_rule(width, th));
        body.push(vec![
            span("a accept", th.border),
            span(" │ ", th.border),
            span("e edit step", th.border),
            span(" │ ", th.border),
            span("ctrl+p", th.border),
        ]);
    } else {
        // The margin note, when there is room for decoration (`1c`).
        if model.decoration() {
            let mut note = vec![pad(
                inner.saturating_sub("parity first,".len() as u16 + 2),
                None,
            )];
            note.push(span("parity first,", th.secondary));
            body.push(note);
            let mut note = vec![pad(
                inner.saturating_sub("speed after".len() as u16 + 2),
                None,
            )];
            note.push(span("speed after", th.secondary));
            body.push(note);
        } else {
            body.push(Vec::new());
        }
        body.push(surface_rule(width, th));
        // One footer row: the tally and its meter left, the keys right (`1c`).
        let cells = 12u16;
        let filled = ((current as f64 / total.max(1) as f64) * cells as f64).round() as usize;
        let left = vec![
            span("constructio ", th.muted),
            span(roman(current.min(total)), th.primary),
            span(" / ", th.border),
            span(format!("{}  ", roman(total)), th.muted),
            span("━".repeat(filled), th.primary),
            span("━".repeat(cells as usize - filled), th.border),
        ];
        let right = vec![
            span("a accept", th.border),
            span(" │ ", th.border),
            span("e edit step", th.border),
            span(" │ ", th.border),
            span("ctrl+p", th.border),
        ];
        let gap = inner
            .saturating_sub(run_width(&left))
            .saturating_sub(run_width(&right))
            .max(1);
        let mut footer = left;
        footer.push(pad(gap, None));
        footer.extend(right);
        body.push(footer);
    }

    Surface::new(width, th)
        .title(vec![
            span("DISEGNO", th.primary),
            span(" · ", th.border),
            span("IMPLEMENTATION PLAN", th.muted),
        ])
        .rows(body)
        .lines()
}

/// Roman numerals for plan positions. Plans are short; five is plenty.
pub fn roman(value: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = value;
    let mut out = String::new();
    for (amount, numeral) in TABLE {
        while left >= amount {
            out.push_str(numeral);
            left -= amount;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_screen(Screen::Plan);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn roman_numerals_count_the_steps() {
        assert_eq!(roman(1), "I");
        assert_eq!(roman(3), "III");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(5), "V");
        assert_eq!(roman(9), "IX");
        assert_eq!(roman(14), "XIV");
        assert_eq!(roman(0), "");
    }

    #[test]
    fn the_sheet_names_itself_and_numbers_its_steps_in_a_gutter() {
        let m = model(120);
        let rows = lines(&m);
        assert!(text(&rows[0]).contains("╭─ DISEGNO · IMPLEMENTATION PLAN ─"));
        for (index, step) in m.plan.iter().enumerate() {
            let drawn = text(&rows[index + 1]);
            assert!(
                drawn.starts_with(&format!("│ {:>3} ", step.numeral)),
                "{drawn}"
            );
        }
    }

    #[test]
    fn every_step_carries_its_state_glyph() {
        let m = model(120);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows[1].contains("  I ✓ "), "{}", rows[1]);
        assert!(rows[3].contains("III ◉ "), "{}", rows[3]);
        assert!(rows[4].contains(" IV ○ "), "{}", rows[4]);
    }

    #[test]
    fn the_footer_counts_the_step_in_hand_with_a_meter_and_the_keys() {
        let m = model(120);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let footer = rows
            .iter()
            .find(|row| row.contains("constructio"))
            .expect("footer");
        assert!(footer.contains("constructio III / V"), "{footer}");
        assert!(footer.contains('━'), "{footer}");
        assert!(
            footer.contains("a accept │ e edit step │ ctrl+p"),
            "{footer}"
        );
    }

    #[test]
    fn the_compass_is_decoration_and_is_dropped_below_a_hundred_columns() {
        let wide: Vec<String> = lines(&model(120)).iter().map(text).collect();
        assert!(wide.iter().any(|row| row.contains('╲')), "compass at 120");

        let narrow: Vec<String> = lines(&model(80)).iter().map(text).collect();
        assert!(
            !narrow.iter().any(|row| row.contains('╲')),
            "no compass at 80"
        );
    }

    #[test]
    fn the_compass_never_reaches_the_panel_label() {
        let m = model(120);
        let top = text(&lines(&m)[0]);
        assert!(top.contains("DISEGNO · IMPLEMENTATION PLAN"), "{top}");
        assert!(!top.contains('╲'), "{top}");
    }

    #[test]
    fn the_sheet_is_row_exact_and_never_wider_than_the_measure_plus_its_gutters() {
        for width in [80u16, 100, 120, 160] {
            let expected = width.min(MEASURE + 12);
            for row in lines(&model(width)) {
                assert_eq!(run_width(&row.spans), expected, "at {width}");
            }
        }
    }

    #[test]
    fn the_footer_offers_the_plans_own_keys() {
        let rows: Vec<String> = lines(&model(120)).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("a accept")));
        assert!(rows.iter().any(|row| row.contains("e edit step")));
    }

    #[test]
    fn the_margin_note_rides_the_sheet_when_there_is_room() {
        let rows: Vec<String> = lines(&model(120)).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("parity first,")));
        assert!(rows.iter().any(|row| row.contains("speed after")));

        let narrow: Vec<String> = lines(&model(80)).iter().map(text).collect();
        assert!(!narrow.iter().any(|row| row.contains("parity first,")));
    }
}
