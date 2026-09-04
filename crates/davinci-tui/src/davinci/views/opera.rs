//! The working line: `◜ Pondering… (12s · ↓ 423 tokens · thinking with high
//! effort)`.
//!
//! One row, pinned directly above the composer, that says a turn is under way
//! and what it has cost so far. The Studio ledger says *what* the turn is
//! doing; this says *that* it is still doing it, and it stays put while the
//! transcript scrolls under it.
//!
//! It spins off the same 250ms clock as every other mark in the shell — the
//! same frame index, so the Studio ledger and this row move as one thing, not
//! two (design.md §8). Under `--no-animation` both freeze on `◉`.
//!
//! The parenthetical is dropped from the right as the window narrows: effort
//! first, then the token count, until only the verb is left.

use ratatui::text::Line;

use crate::davinci::model::Model;
use crate::davinci::ui::{run_width, span, truncate_run};

use super::chrome::thousands;

/// Row count, known before rendering, so the shell can budget for it.
pub fn height(model: &Model) -> usize {
    if model.working.is_some() {
        1
    } else {
        0
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(working) = &model.working else {
        return Vec::new();
    };

    let mut spans = vec![
        span(
            format!("{} ", th.spinner(model.tick, model.animate)),
            th.primary,
        ),
        span(format!("{}… ", working.verb()), th.primary),
    ];

    // Every field the parenthetical can hold, longest last, so the row can be
    // shortened by dropping from the end rather than by re-measuring.
    let mut facts = vec![format!("{}s", working.seconds)];
    if working.tokens > 0 {
        facts.push(format!("↓ {} tokens", thousands(working.tokens)));
    }
    if let Some(effort) = &working.thinking {
        facts.push(format!("thinking with {effort} effort"));
    }

    while !facts.is_empty() {
        let tail = tail_spans(model, &facts);
        if run_width(&spans) + run_width(&tail) <= model.width.saturating_sub(2) {
            spans.extend(tail);
            break;
        }
        facts.pop();
    }

    // A window too narrow even for the verb still gets a row that fits: the
    // spinner is the part that matters, and it is first.
    vec![Line::from(truncate_run(spans, model.width))]
}

fn tail_spans(model: &Model, facts: &[String]) -> Vec<ratatui::text::Span<'static>> {
    let th = &model.theme;
    let mut tail = vec![span("(", th.border)];
    for (index, fact) in facts.iter().enumerate() {
        if index > 0 {
            tail.push(span(" · ", th.border));
        }
        tail.push(span(fact.clone(), th.muted));
    }
    tail.push(span(")", th.border));
    tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Working;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        )
    }

    fn working() -> Working {
        Working {
            seconds: 12,
            tokens: 423,
            thinking: Some("high".into()),
        }
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn nothing_is_drawn_between_turns() {
        let m = model(120);
        assert!(lines(&m).is_empty());
        assert_eq!(height(&m), 0);
    }

    #[test]
    fn a_running_turn_states_its_verb_elapsed_tokens_and_effort() {
        let mut m = model(120);
        m.working = Some(working());
        let rows = lines(&m);
        assert_eq!(rows.len(), 1);
        assert_eq!(height(&m), 1);
        assert_eq!(
            text(&rows[0]),
            "◜ Measuring… (12s · ↓ 423 tokens · thinking with high effort)"
        );
    }

    #[test]
    fn the_verb_turns_over_every_three_seconds_and_never_repeats_adjacently() {
        let mut m = model(120);
        let mut seen = Vec::new();
        for seconds in [0u64, 2, 3, 6, 9] {
            m.working = Some(Working {
                seconds,
                ..working()
            });
            seen.push(text(&lines(&m)[0]).split('…').next().unwrap().to_string());
        }
        assert_eq!(seen[0], seen[1], "the word holds for three seconds");
        assert_ne!(seen[1], seen[2]);
        assert_ne!(seen[2], seen[3]);
        assert_ne!(seen[3], seen[4]);
    }

    #[test]
    fn it_spins_on_the_shared_clock_and_freezes_without_animation() {
        let mut m = model(120);
        m.working = Some(working());
        for (tick, frame) in [(0u64, '◜'), (1, '◝'), (2, '◞'), (3, '◟')] {
            m.tick = tick;
            assert!(text(&lines(&m)[0]).starts_with(frame), "{tick}");
        }
        m.animate = false;
        assert!(text(&lines(&m)[0]).starts_with('◉'));
    }

    #[test]
    fn a_silent_model_states_only_the_elapsed_time() {
        let mut m = model(120);
        m.working = Some(Working::new());
        assert_eq!(text(&lines(&m)[0]), "◜ Pondering… (0s)");
    }

    #[test]
    fn the_tail_is_dropped_from_the_right_as_the_window_narrows() {
        let mut m = model(48);
        m.working = Some(working());
        let drawn = text(&lines(&m)[0]);
        assert!(drawn.contains("423 tokens"), "{drawn}");
        assert!(!drawn.contains("effort"), "{drawn}");

        m.width = 26;
        let drawn = text(&lines(&m)[0]);
        assert_eq!(drawn, "◜ Measuring… (12s)");
    }

    #[test]
    fn it_never_overruns_the_window() {
        let mut m = model(20);
        m.working = Some(working());
        for width in 12..90u16 {
            m.width = width;
            let rows = lines(&m);
            assert!(
                run_width(&rows[0].spans) <= width,
                "{width}: {}",
                text(&rows[0])
            );
        }
    }
}
