//! The only box allowed mid-turn (design.md §6): a ledger of ✓ / ◉ / ○ steps
//! with the active step's target appended in border color. Below 100 columns it
//! collapses to one line, `⟐ studying <path>` (screen `1g`).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/studio.ex`.

use ratatui::text::Line;

use crate::davinci::model::{Model, Step};
use crate::davinci::theme::State;
use crate::davinci::ui::{clip_ellipsis, span, span_strong, Surface, MEASURE};

pub fn lines(model: &Model, steps: &[Step]) -> Vec<Line<'static>> {
    if model.narrow() {
        collapsed(model, steps)
    } else {
        expanded(model, steps)
    }
}

/// Row count, known before rendering, so the transcript can budget for it.
pub fn height(model: &Model, steps: &[Step]) -> usize {
    if model.narrow() {
        1
    } else {
        steps.len() + 2
    }
}

fn active_step(steps: &[Step]) -> Option<&Step> {
    steps
        .iter()
        .find(|step| step.state == State::Active)
        .or_else(|| steps.first())
}

fn collapsed(model: &Model, steps: &[Step]) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(step) = active_step(steps) else {
        return Vec::new();
    };
    let subject = step.target.clone().unwrap_or_else(|| step.verb.clone());
    vec![Line::from(vec![
        span(
            format!("{} ", th.spinner(model.tick, model.animate)),
            th.primary,
        ),
        span("studying ", th.muted),
        span(
            clip_ellipsis(&subject, model.width.saturating_sub(14)),
            th.secondary,
        ),
    ])]
}

fn expanded(model: &Model, steps: &[Step]) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 6);
    let mut surface = Surface::new(width, th).title(vec![span("STUDIO", th.primary)]);

    for step in steps {
        let glyph = if step.state == State::Active {
            th.spinner(model.tick, model.animate).to_string()
        } else {
            step.state.glyph().to_string()
        };
        let verb_color = if step.state == State::Queued {
            th.muted
        } else {
            th.text
        };
        let mut row = vec![
            span_strong(format!("{glyph} "), th.state_color(step.state), th),
            span(step.verb.clone(), verb_color),
        ];
        if let Some(target) = &step.target {
            row.push(span(" · ", th.border));
            row.push(span(
                clip_ellipsis(target, width.saturating_sub(40)),
                th.border,
            ));
        }
        surface = surface.row(row);
    }

    surface.lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        )
    }

    fn steps() -> Vec<Step> {
        vec![
            Step::new(State::Done, "surveyed workspace", None),
            Step::new(State::Done, "traced request pipeline", None),
            Step::new(
                State::Active,
                "examining session persistence",
                Some("davinci-session\\src\\store.rs"),
            ),
            Step::new(State::Queued, "verify provider abstraction", None),
        ]
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn at_a_hundred_columns_it_is_a_box_with_one_row_per_step() {
        let m = model(100);
        let rows = lines(&m, &steps());
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.len(), height(&m, &steps()));
        assert!(text(&rows[0]).starts_with("╭─ STUDIO ─"));
        assert!(text(&rows[1]).contains("✓ surveyed workspace"));
        assert!(text(&rows[4]).contains("○ verify provider abstraction"));
    }

    #[test]
    fn the_active_step_carries_the_spinner_and_its_target() {
        let mut m = model(100);
        let steps = steps();
        for (tick, frame) in [(0u64, '◜'), (1, '◝'), (2, '◞'), (3, '◟')] {
            m.tick = tick;
            let drawn = text(&lines(&m, &steps)[3]);
            assert!(drawn.starts_with(&format!("│ {frame} ")), "{drawn}");
        }
        let drawn = text(&lines(&m, &steps)[3]);
        assert!(drawn.contains("examining session persistence"));
        assert!(drawn.contains("store.rs"), "{drawn}");
    }

    #[test]
    fn the_spinner_freezes_under_no_animation() {
        let mut m = model(100);
        m.animate = false;
        m.tick = 3;
        let drawn = text(&lines(&m, &steps())[3]);
        assert!(drawn.starts_with("│ ◉ "), "{drawn}");
    }

    #[test]
    fn below_a_hundred_columns_it_collapses_to_one_line() {
        let m = model(80);
        let rows = lines(&m, &steps());
        assert_eq!(rows.len(), 1);
        assert_eq!(height(&m, &steps()), 1);
        let drawn = text(&rows[0]);
        assert!(drawn.contains("studying "), "{drawn}");
        assert!(drawn.contains("store.rs"), "{drawn}");
        assert!(!drawn.contains('╭'), "no box below 100 columns: {drawn}");
    }

    #[test]
    fn a_collapsed_ledger_with_no_active_step_falls_back_to_the_first() {
        let m = model(80);
        let steps = vec![Step::new(State::Done, "surveyed workspace", None)];
        let drawn = text(&lines(&m, &steps)[0]);
        assert!(drawn.contains("surveyed workspace"), "{drawn}");
    }

    #[test]
    fn an_empty_ledger_draws_nothing_when_collapsed() {
        assert!(lines(&model(80), &[]).is_empty());
    }

    #[test]
    fn the_box_never_grows_past_the_measure() {
        let m = model(200);
        let rows = lines(&m, &steps());
        let width = crate::davinci::ui::run_width(&rows[0].spans);
        assert_eq!(width, MEASURE + 6);
    }
}
