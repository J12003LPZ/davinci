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
use crate::davinci::ui::{pad, run_width, span, span_strong, ticks, Surface, MEASURE};

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
    let done = model
        .plan
        .iter()
        .filter(|step| step.state == State::Done)
        .count();
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

    body.push(Vec::new());
    let mut footer = vec![
        span("constructio ", th.muted),
        span(roman(done.min(total)), th.text),
        span(" / ", th.border),
        span(format!("{}  ", roman(total)), th.muted),
    ];
    footer.extend(ticks(done, total, 24, th));
    body.push(footer);
    body.push(vec![
        span("a accept", th.border),
        span(" · ", th.border),
        span("e edit step", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]);

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
    fn the_footer_reads_constructio_with_a_tick_meter() {
        let m = model(120);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let footer = rows
            .iter()
            .find(|row| row.contains("constructio"))
            .expect("footer");
        assert!(footer.contains("constructio II / V"), "{footer}");
        assert!(footer.contains('━') && footer.contains('·'), "{footer}");
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
    fn the_footer_states_its_exit() {
        let rows: Vec<String> = lines(&model(120)).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("esc close")));
        assert!(rows.iter().any(|row| row.contains("a accept")));
    }
}
