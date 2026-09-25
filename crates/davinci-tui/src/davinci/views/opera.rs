//! Claude Code-style working line pinned above the composer.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::ui::{run_width, span, truncate_run};

use super::chrome::thousands;

pub const SPINNER_FRAMES: [&str; 6] = ["·", "✢", "*", "✶", "✻", "✽"];

pub fn height(model: &Model) -> usize {
    usize::from(model.working.is_some())
}

fn frame(tick: u64, animate: bool) -> &'static str {
    if !animate {
        return "✻";
    }
    let step = (tick % 10) as usize;
    SPINNER_FRAMES[if step < 6 { step } else { 10 - step }]
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let Some(working) = &model.working else {
        return Vec::new();
    };

    let mut spans = vec![span(
        format!("{} ", frame(model.tick, model.animate)),
        cc.claude,
    )];
    spans.extend(shimmer(
        working.verb(),
        model.tick,
        model.animate,
        cc.claude,
        cc.claude_shimmer,
    ));
    spans.push(span("… ", cc.claude));

    let mut facts = vec![format!("{}s", working.seconds)];
    if working.tokens > 0 {
        facts.push(format!("↓ {} tokens", thousands(working.tokens)));
    }
    let reasoning = match (&working.thinking, working.reasoning, working.thought_for) {
        (Some(effort), true, _) => Some(format!("thinking with {effort} effort")),
        (_, false, Some(seconds)) => Some(format!("thought for {seconds}s")),
        _ => None,
    };
    let lively = working.reasoning;
    if let Some(reasoning) = reasoning {
        facts.push(reasoning);
    }

    while !facts.is_empty() {
        let mut tail = vec![span("(", cc.inactive)];
        for (index, fact) in facts.iter().enumerate() {
            if index > 0 {
                tail.push(span(" · ", cc.inactive));
            }
            let last = index == facts.len() - 1;
            let color = if last && lively && model.tick % 2 == 0 {
                cc.inactive_shimmer
            } else {
                cc.inactive
            };
            tail.push(span(fact.clone(), color));
        }
        tail.push(span(")", cc.inactive));
        if run_width(&spans) + run_width(&tail) <= model.width.saturating_sub(2) {
            spans.extend(tail);
            break;
        }
        facts.pop();
    }
    vec![Line::from(truncate_run(spans, model.width))]
}

fn shimmer(word: &str, tick: u64, animate: bool, base: Color, light: Color) -> Vec<Span<'static>> {
    let chars: Vec<char> = word.chars().collect();
    if !animate || chars.is_empty() {
        return vec![span(word.to_string(), base)];
    }
    let cycle = chars.len() + 3;
    let start = (tick as usize) % cycle;
    let mut out: Vec<Span<'static>> = Vec::new();
    for (index, ch) in chars.iter().enumerate() {
        let lit = index + 3 >= start && index < start;
        let color = if lit { light } else { base };
        match out.last_mut() {
            Some(last) if last.style.fg == Some(color) => {
                last.content = format!("{}{ch}", last.content).into();
            }
            _ => out.push(span(ch.to_string(), color)),
        }
    }
    out
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
            reasoning: true,
            ..Working::default()
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
            "◜ Measuring… (esc to interrupt · 12s · ↓ 423 tokens · thinking with high effort)"
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
    fn a_silent_model_keeps_the_interrupt_hint_and_elapsed_time() {
        let mut m = model(120);
        m.working = Some(Working::new());
        assert_eq!(text(&lines(&m)[0]), "◜ Pondering… (esc to interrupt · 0s)");
    }

    #[test]
    fn the_tail_is_dropped_from_the_right_as_the_window_narrows() {
        let mut m = model(64);
        m.working = Some(working());
        let drawn = text(&lines(&m)[0]);
        assert!(drawn.contains("423 tokens"), "{drawn}");
        assert!(!drawn.contains("effort"), "{drawn}");

        m.width = 34;
        let drawn = text(&lines(&m)[0]);
        assert_eq!(drawn, "◜ Measuring… (esc to interrupt)");
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
