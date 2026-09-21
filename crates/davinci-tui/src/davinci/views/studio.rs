//! Compact task checklist inside the conversation. Each task stays readable
//! at ordinary terminal widths; raw orchestration metadata belongs in inspectors.
use crate::davinci::model::{Model, Step};
use crate::davinci::theme::State;
use crate::davinci::ui::{self, clip_ellipsis, span, span_strong, MEASURE};
use ratatui::text::Line;

pub fn lines(model: &Model, steps: &[Step]) -> Vec<Line<'static>> {
    if steps.is_empty() {
        return Vec::new();
    }
    let th = &model.theme;
    if model.width < 60 {
        let task = steps
            .iter()
            .find(|step| step.state == State::Active)
            .unwrap_or(&steps[0]);
        let text = format!(
            "{} {}{}",
            task.state.glyph(),
            task.verb,
            task.target
                .as_ref()
                .map(|target| format!(" · {target}"))
                .unwrap_or_default()
        );
        return vec![Line::from(span(clip_ellipsis(&text, model.width), th.text))];
    }
    let complete = steps
        .iter()
        .filter(|step| step.state == State::Done)
        .count();
    let mut rows = vec![Line::from(span(
        format!("  Tasks · {complete}/{} complete", steps.len()),
        th.muted,
    ))];
    for step in steps {
        let glyph = if step.state == State::Active {
            th.spinner(model.tick, model.animate).to_string()
        } else {
            step.state.glyph().to_string()
        };
        let mut spans = vec![
            span("  ", th.muted),
            span_strong(format!("{glyph} "), th.state_color(step.state), th),
            span(
                step.verb.clone(),
                if step.state == State::Queued {
                    th.muted
                } else {
                    th.text
                },
            ),
        ];
        if let Some(target) = &step.target {
            spans.push(span(format!(" · {target}"), th.muted));
        }
        rows.push(Line::from(ui::truncate_run(
            spans,
            model.width.min(MEASURE + 6),
        )));
    }
    rows
}

pub fn height(model: &Model, steps: &[Step]) -> usize {
    if steps.is_empty() {
        0
    } else if model.width < 60 {
        1
    } else {
        steps.len() + 1
    }
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
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.len(), height(&m, &steps()));
        assert!(text(&rows[0]).starts_with("  Tasks · 2/4 complete"));
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
            assert!(drawn.starts_with(&format!("  {frame} ")), "{drawn}");
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
        assert!(drawn.starts_with("  ◉ "), "{drawn}");
    }

    #[test]
    fn below_a_hundred_columns_it_collapses_to_one_line() {
        let m = model(50);
        let rows = lines(&m, &steps());
        assert_eq!(rows.len(), 1);
        assert_eq!(height(&m, &steps()), 1);
        let drawn = text(&rows[0]);
        assert!(drawn.contains("examining session"), "{drawn}");
        assert!(drawn.ends_with('…'), "{drawn}");
        assert!(!drawn.contains('╭'), "no box below 100 columns: {drawn}");
    }

    #[test]
    fn a_collapsed_ledger_with_no_active_step_falls_back_to_the_first() {
        let m = model(50);
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
        assert!(width <= MEASURE + 6);
        assert!(rows
            .iter()
            .all(|row| crate::davinci::ui::run_width(&row.spans) <= MEASURE + 6));
    }
}
