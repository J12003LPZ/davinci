//! Claude Code-style working line pinned above the composer.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, GLINT_TICKS};
use crate::davinci::ui::{breath, mix, run_width, span, truncate_run};

use super::chrome::key_label;
use super::transcript::{duration_words, ELBOW};

pub const SPINNER_FRAMES: [&str; 6] = ["·", "✢", "*", "✶", "✻", "✽"];

/// Rows the tip needs below it to leave the transcript room to breathe.
const TIP_MIN_HEIGHT: u16 = 14;
/// Narrower than this the tip would be clipped to a stub, so it is dropped.
const TIP_MIN_WIDTH: u16 = 40;

pub fn height(model: &Model) -> usize {
    lines(model).len()
}

/// `423`, `1.2k`, `18.5k`, `1.2m`: one decimal while it carries information,
/// as the reference's counter does.
fn token_count(value: u64) -> String {
    let scaled = |unit: f64, suffix: &str| {
        let text = format!("{:.1}", value as f64 / unit);
        format!("{}{suffix}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match value {
        0..=999 => value.to_string(),
        1_000..=999_949 => scaled(1_000.0, "k"),
        _ => scaled(1_000_000.0, "m"),
    }
}

/// One hint for the turn, rotated by the turn count so a session cycles
/// through them and a frame stays reproducible. Every key is read from the
/// loaded keymap, so a rebinding changes the tip with it.
fn tip(model: &Model, turn: u64) -> String {
    let key = |action: &str, fallback: &str| key_label(model, action).unwrap_or(fallback.into());
    let tips = [
        format!(
            "Hit {} to cycle between manual mode, accept edits mode, and plan mode",
            key("app.permissions.cycle", "shift+tab")
        ),
        format!("Hit {} to interrupt the turn", key("app.interrupt", "esc")),
        format!(
            "Hit {} to open the full transcript",
            key("davinci.transcript.open", "ctrl+o")
        ),
        format!(
            "Hit {} to show verbose tool output",
            key("davinci.tools.expand", "alt+o")
        ),
        "Use /model to switch models".to_string(),
        "Use /effort to change how hard the model thinks".to_string(),
        "Use /context to see what is filling the context window".to_string(),
        "Start a line with ! to run a shell command, or type @ to pick a file".to_string(),
    ];
    tips[turn as usize % tips.len()].clone()
}

/// Ghostty draws `✳` where other non-mac terminals' fonts fall back for it,
/// so it gets the reference's own set (`✳` and a closing `*`).
const GHOSTTY_FRAMES: [&str; 6] = ["·", "✢", "✳", "✶", "✻", "*"];

fn frames() -> &'static [&'static str; 6] {
    use std::sync::OnceLock;
    static SET: OnceLock<bool> = OnceLock::new();
    let ghostty = *SET.get_or_init(|| {
        std::env::var("TERM_PROGRAM").is_ok_and(|term| term.eq_ignore_ascii_case("ghostty"))
    });
    if ghostty {
        &GHOSTTY_FRAMES
    } else {
        &SPINNER_FRAMES
    }
}

/// Out and back over the glyphs, the last one dwelling for two ticks as in
/// the reference's twelve-step loop.
fn frame_of(set: &'static [&'static str; 6], tick: u64) -> &'static str {
    let step = (tick % 12) as usize;
    set[if step < 6 { step } else { 11 - step }]
}

fn frame(tick: u64, animate: bool) -> &'static str {
    if !animate {
        return "✻";
    }
    frame_of(frames(), tick)
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let Some(working) = &model.working else {
        return Vec::new();
    };

    // A turn that has gone quiet warms from the accent toward the error ink,
    // so a hung request is visible before anyone wonders. It is a change of
    // color, not motion, so reduced motion keeps it; once fully stalled a
    // text cue says the same thing for palettes with no red to show.
    let stall = working.stall();
    let accent = mix(cc.claude, cc.error, stall);
    let accent_light = mix(cc.claude_shimmer, cc.error, stall);

    let mut spans = vec![span(
        format!("{} ", frame(model.tick, model.animate)),
        accent,
    )];
    spans.extend(shimmer(
        working.verb(),
        model.tick,
        model.animate,
        accent,
        accent_light,
    ));
    spans.push(span("… ", accent));

    let mut facts = vec![(duration_words(working.seconds), cc.inactive)];
    if stall >= 1.0 {
        facts.push((
            format!("no response for {}s", working.quiet_seconds()),
            cc.error,
        ));
    }
    if working.tokens > 0 {
        // Crossing a thousand earns a brief glint that fades over a few ticks.
        let glint = if model.animate {
            f32::from(working.glint) / f32::from(GLINT_TICKS)
        } else {
            0.0
        };
        facts.push((
            format!("↓ {} tokens", token_count(working.displayed_tokens())),
            mix(cc.inactive, cc.claude_shimmer, glint),
        ));
    }
    let reasoning = match (&working.thinking, working.reasoning, working.thought_for) {
        (Some(effort), true, _) => Some(format!("thinking with {effort} effort")),
        (_, false, Some(seconds)) => Some(format!("thought for {seconds}s")),
        _ => None,
    };
    if let Some(reasoning) = reasoning {
        // Live reasoning breathes on a two-second cycle instead of blinking.
        let color = if working.reasoning && model.animate {
            mix(cc.inactive, cc.inactive_shimmer, breath(model.tick, 8))
        } else {
            cc.inactive
        };
        facts.push((reasoning, color));
    }

    while !facts.is_empty() {
        let mut tail = vec![span("(", cc.inactive)];
        for (index, (fact, color)) in facts.iter().enumerate() {
            if index > 0 {
                tail.push(span(" · ", cc.inactive));
            }
            tail.push(span(fact.clone(), *color));
        }
        tail.push(span(")", cc.inactive));
        if run_width(&spans) + run_width(&tail) <= model.width.saturating_sub(2) {
            spans.extend(tail);
            break;
        }
        facts.pop();
    }
    let mut rows = vec![Line::from(truncate_run(spans, model.width))];

    // The tip rides under the working line for a plain turn. Compaction and
    // an interrupt in flight have their own message, and a short or narrow
    // window keeps its rows for the transcript.
    if working.label.is_none()
        && !working.interrupting
        && model.height >= TIP_MIN_HEIGHT
        && model.width >= TIP_MIN_WIDTH
    {
        let tip = vec![
            span(ELBOW, cc.inactive),
            span(
                format!("Tip: {}", tip(model, working.verb_seed)),
                cc.inactive,
            ),
        ];
        rows.push(Line::from(truncate_run(tip, model.width)));
    }
    rows
}

/// A band of light sweeping left to right across `word`, one cell per tick.
/// The band has a bright core and soft shoulders on truecolor terminals; on
/// a palette without in-between shades the shoulders round off and it draws
/// as the classic three-cell highlight.
pub(super) fn shimmer(
    word: &str,
    tick: u64,
    animate: bool,
    base: Color,
    light: Color,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = word.chars().collect();
    if !animate || chars.is_empty() {
        return vec![span(word.to_string(), base)];
    }
    // Two cells of dark after the band leaves, so the next pass enters
    // from off the left edge instead of popping in mid-word.
    let cycle = chars.len() + 5;
    let core = (tick as usize % cycle) as isize - 2;
    let mut out: Vec<Span<'static>> = Vec::new();
    for (index, ch) in chars.iter().enumerate() {
        let glow = match (index as isize - core).unsigned_abs() {
            0 => 1.0,
            1 => 0.6,
            2 => 0.2,
            _ => 0.0,
        };
        let color = mix(base, light, glow);
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
    use crate::davinci::model::{Working, STALL_AFTER, STALL_RAMP};
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
            .collect::<String>()
            .replace('\u{a0}', " ")
    }

    #[test]
    fn token_counts_keep_one_decimal_like_the_reference() {
        for (value, drawn) in [
            (0u64, "0"),
            (999, "999"),
            (1_000, "1k"),
            (1_234, "1.2k"),
            (18_500, "18.5k"),
            (99_960, "100k"),
            (999_949, "999.9k"),
            (999_950, "1m"),
            (1_260_000, "1.3m"),
        ] {
            assert_eq!(token_count(value), drawn, "{value}");
        }
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
        assert_eq!(rows.len(), 2);
        assert_eq!(height(&m), 2);
        assert_eq!(
            text(&rows[0]),
            "· Boondoggling… (12s · ↓ 423 tokens · thinking with high effort)"
        );
    }

    #[test]
    fn elapsed_time_reads_seconds_then_minutes_then_hours() {
        let mut m = model(120);
        for (seconds, drawn) in [
            (0u64, "(0s)"),
            (59, "(59s)"),
            (60, "(1m 0s)"),
            (222, "(3m 42s)"),
            (3599, "(59m 59s)"),
            (3600, "(1h 0m)"),
            (7500, "(2h 5m)"),
        ] {
            m.working = Some(Working {
                seconds,
                ..Working::new()
            });
            assert!(text(&lines(&m)[0]).ends_with(drawn), "{seconds}");
        }
        m.working = Some(Working {
            seconds: 222,
            tokens: 18_500,
            ..Working::new()
        });
        assert_eq!(
            text(&lines(&m)[0]),
            "· Boondoggling… (3m 42s · ↓ 18.5k tokens)"
        );
    }

    #[test]
    fn a_plain_turn_puts_a_dim_tip_under_the_working_line() {
        let mut m = model(120);
        m.working = Some(working());
        let rows = lines(&m);
        assert_eq!(
            text(&rows[1]),
            "  ⎿  Tip: Hit shift+tab to cycle between manual mode, accept edits mode, and plan mode"
        );
        let cc = m.theme.cc();
        assert!(rows[1]
            .spans
            .iter()
            .all(|s| s.style.fg == Some(cc.inactive)));
    }

    #[test]
    fn the_tip_rotates_with_the_turn_and_follows_rebinding() {
        let mut m = model(120);
        let tips: Vec<String> = (0..8)
            .map(|turn| {
                m.working = Some(Working {
                    verb_seed: turn,
                    ..working()
                });
                text(&lines(&m)[1])
            })
            .collect();
        let distinct: std::collections::HashSet<_> = tips.iter().collect();
        assert_eq!(distinct.len(), 8, "{tips:?}");
        m.working = Some(Working {
            verb_seed: 8,
            ..working()
        });
        assert_eq!(text(&lines(&m)[1]), tips[0], "wraps after the last tip");
        assert!(tips[2].ends_with("Hit ctrl+o to open the full transcript"));
        assert!(tips[3].ends_with("Hit alt+o to show verbose tool output"));
    }

    #[test]
    fn the_tip_is_dropped_when_the_window_is_short_or_narrow_or_the_work_is_special() {
        let mut m = model(120);
        m.working = Some(working());
        m.height = 13;
        assert_eq!(lines(&m).len(), 1);
        assert_eq!(height(&m), 1);
        m.height = 14;
        assert_eq!(lines(&m).len(), 2);
        m.width = 39;
        assert_eq!(lines(&m).len(), 1);
        m.width = 40;
        let rows = lines(&m);
        assert_eq!(rows.len(), 2);
        assert!(run_width(&rows[1].spans) <= 40, "{}", text(&rows[1]));
        assert!(text(&rows[1]).ends_with('…'));
        m.working.as_mut().unwrap().interrupting = true;
        assert_eq!(lines(&m).len(), 1);
        m.working = Some(Working {
            label: Some("Compacting conversation"),
            ..working()
        });
        assert_eq!(lines(&m).len(), 1);
    }

    #[test]
    fn the_verb_stays_stable_for_the_turn() {
        let mut m = model(120);
        let mut seen = Vec::new();
        for seconds in [0u64, 2, 3, 6, 9] {
            m.working = Some(Working {
                seconds,
                ..working()
            });
            seen.push(text(&lines(&m)[0]).split('…').next().unwrap().to_string());
        }
        assert!(seen.iter().all(|verb| verb == &seen[0]));
        m.working.as_mut().unwrap().verb_seed = 1;
        assert!(text(&lines(&m)[0]).starts_with("· Levitating"));
        m.working.as_mut().unwrap().interrupting = true;
        assert!(text(&lines(&m)[0]).contains("Interrupting"));
    }

    #[test]
    fn ghostty_gets_its_own_glyph_set_and_both_loop_over_twelve_ticks() {
        let ghost: Vec<&str> = (0..12).map(|t| frame_of(&GHOSTTY_FRAMES, t)).collect();
        assert_eq!(
            ghost,
            ["·", "✢", "✳", "✶", "✻", "*", "*", "✻", "✶", "✳", "✢", "·"]
        );
        assert_eq!(frame_of(&GHOSTTY_FRAMES, 12), frame_of(&GHOSTTY_FRAMES, 0));
    }

    #[test]
    fn it_spins_on_the_shared_clock_and_freezes_without_animation() {
        let mut m = model(120);
        m.working = Some(working());
        for (tick, frame) in [
            (0u64, '·'),
            (1, '✢'),
            (2, '*'),
            (3, '✶'),
            (5, '✽'),
            (6, '✽'),
            (9, '*'),
            (11, '·'),
            (12, '·'),
        ] {
            m.tick = tick;
            assert!(text(&lines(&m)[0]).starts_with(frame), "{tick}");
        }
        m.animate = false;
        assert!(text(&lines(&m)[0]).starts_with('✻'));
    }

    #[test]
    fn the_token_count_rolls_up_one_tick_at_a_time_and_never_overshoots() {
        let mut m = model(120);
        m.working = Some(Working {
            tokens: 900,
            ..Working::new()
        });
        let mut seen = Vec::new();
        for _ in 0..20 {
            m.advance_tick();
            seen.push(m.working.as_ref().unwrap().displayed_tokens());
        }
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
        assert!(seen[0] > 0 && seen[0] < 900, "{seen:?}");
        assert_eq!(*seen.last().unwrap(), 900, "{seen:?}");
        m.working.as_mut().unwrap().tokens = 5;
        assert_eq!(m.working.as_ref().unwrap().displayed_tokens(), 5);
        m.working.as_mut().unwrap().seconds = 3;
        assert!(text(&lines(&m)[0]).contains("↓ 5 tokens"));
    }

    #[test]
    fn the_token_count_is_exact_when_animation_is_off() {
        let mut m = model(120);
        m.animate = false;
        m.working = Some(working());
        m.advance_tick();
        assert!(text(&lines(&m)[0]).contains("↓ 423 tokens"));
    }

    #[test]
    fn the_shimmer_band_has_a_bright_core_and_soft_shoulders() {
        let cc = model(80).theme.cc();
        let spans = shimmer("Boondoggling", 6, true, cc.claude, cc.claude_shimmer);
        let word: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(word, "Boondoggling");
        let inks: Vec<_> = spans.iter().map(|span| span.style.fg.unwrap()).collect();
        assert!(inks.contains(&cc.claude_shimmer));
        assert!(inks.contains(&cc.claude));
        assert!(
            inks.iter()
                .any(|ink| *ink != cc.claude && *ink != cc.claude_shimmer),
            "{inks:?}"
        );
        let still = shimmer("Boondoggling", 6, false, cc.claude, cc.claude_shimmer);
        assert_eq!(still.len(), 1);
    }

    #[test]
    fn a_256_color_shimmer_keeps_the_classic_three_cell_band() {
        let cc = Theme::da_vinci(ColorDepth::Ansi256, false).cc();
        let spans = shimmer("abcdefgh", 5, true, cc.claude, cc.claude_shimmer);
        let lit: String = spans
            .iter()
            .filter(|span| span.style.fg == Some(cc.claude_shimmer))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(lit, "cde");
    }

    #[test]
    fn a_quiet_turn_warms_the_spinner_toward_the_error_ink() {
        let mut m = model(120);
        let cc = m.theme.cc();
        m.working = Some(Working {
            tokens: 50,
            ..Working::new()
        });
        let spinner = |m: &Model| lines(m)[0].spans[0].style.fg;
        m.advance_tick();
        assert_eq!(spinner(&m), Some(cc.claude));
        for _ in 0..STALL_AFTER {
            m.advance_tick();
        }
        assert_eq!(
            spinner(&m),
            Some(cc.claude),
            "still inside the grace period"
        );
        for _ in 0..STALL_RAMP / 2 {
            m.advance_tick();
        }
        let warming = spinner(&m);
        assert_ne!(warming, Some(cc.claude));
        assert_ne!(warming, Some(cc.error));
        for _ in 0..STALL_RAMP {
            m.advance_tick();
        }
        assert_eq!(spinner(&m), Some(cc.error));

        let drawn = text(&lines(&m)[0]);
        assert!(drawn.contains("no response for 17s"), "{drawn}");

        // One new token and it is lively again.
        m.working.as_mut().unwrap().tokens += 1;
        m.advance_tick();
        assert_eq!(spinner(&m), Some(cc.claude));
        assert!(!text(&lines(&m)[0]).contains("no response"));

        // Any agent event counts, even one that adds no tokens.
        for _ in 0..STALL_AFTER + STALL_RAMP {
            m.advance_tick();
        }
        m.working.as_mut().unwrap().pulse();
        assert_eq!(spinner(&m), Some(cc.claude));

        // Reduced motion still warns: a color change is not motion.
        m.animate = false;
        for _ in 0..STALL_AFTER + STALL_RAMP + 1 {
            m.advance_tick();
        }
        assert_eq!(spinner(&m), Some(cc.error));
        assert!(text(&lines(&m)[0]).contains("no response"));
    }

    #[test]
    fn a_no_color_terminal_still_says_the_turn_has_gone_quiet() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::Basic, true), 120, 44, true);
        m.working = Some(Working::new());
        for _ in 0..STALL_AFTER + STALL_RAMP + 8 {
            m.advance_tick();
        }
        let drawn = text(&lines(&m)[0]);
        assert!(drawn.contains("no response for 17s"), "{drawn}");
    }

    #[test]
    fn crossing_a_thousand_tokens_glints_then_fades() {
        let mut m = model(120);
        let cc = m.theme.cc();
        m.working = Some(Working {
            seconds: 4,
            tokens: 1_200,
            shown_tokens: Some(990),
            ..Working::new()
        });
        let ink = |m: &Model| {
            lines(m)[0]
                .spans
                .iter()
                .find(|span| span.content.contains("tokens"))
                .and_then(|span| span.style.fg)
        };
        m.advance_tick();
        assert_eq!(ink(&m), Some(cc.claude_shimmer));
        m.advance_tick();
        let fading = ink(&m);
        assert_ne!(fading, Some(cc.claude_shimmer));
        assert_ne!(fading, Some(cc.inactive));
        for _ in 0..GLINT_TICKS {
            m.advance_tick();
        }
        assert_eq!(ink(&m), Some(cc.inactive));
    }

    #[test]
    fn live_reasoning_breathes_instead_of_blinking() {
        let mut m = model(120);
        m.working = Some(working());
        let ink = |m: &Model| {
            lines(m)[0]
                .spans
                .iter()
                .find(|span| span.content.contains("effort"))
                .and_then(|span| span.style.fg)
        };
        let inks: Vec<_> = (0..8u64)
            .map(|tick| {
                m.tick = tick;
                ink(&m)
            })
            .collect();
        let distinct: std::collections::HashSet<_> = inks.iter().collect();
        assert!(distinct.len() >= 4, "a fade, not an on/off blink: {inks:?}");
        m.animate = false;
        assert_eq!(ink(&m), Some(m.theme.cc().inactive));
    }

    #[test]
    fn a_running_tool_or_open_prompt_is_not_a_stall() {
        use crate::davinci::model::Entry;
        use crate::davinci::theme::State;
        let mut m = model(120);
        m.transcript.push(Entry::User("go".into()));
        m.transcript
            .push(Entry::tool(State::Read, "lector", "read a.rs", None));
        m.working = Some(Working::new());
        for _ in 0..STALL_AFTER + STALL_RAMP + 1 {
            m.advance_tick();
        }
        assert_eq!(m.working.as_ref().unwrap().stall(), 0.0);

        // The call finishes; silence now counts.
        m.transcript.pop();
        m.transcript.push(Entry::tool(
            State::Read,
            "lector",
            "read a.rs",
            Some("0.2s"),
        ));
        for _ in 0..STALL_AFTER + STALL_RAMP + 1 {
            m.advance_tick();
        }
        assert_eq!(m.working.as_ref().unwrap().stall(), 1.0);

        // A notice never holds it off.
        m.transcript.push(Entry::notice(State::Done, "trusted"));
        m.advance_tick();
        assert_eq!(m.working.as_ref().unwrap().stall(), 1.0);

        // An open call left by an earlier turn does not either.
        m.transcript
            .push(Entry::tool(State::Read, "lector", "read b.rs", None));
        m.transcript.push(Entry::User("again".into()));
        m.advance_tick();
        assert_eq!(m.working.as_ref().unwrap().stall(), 1.0);

        // An open prompt does.
        m.overlay = Some(crate::davinci::model::Overlay::Ask);
        m.advance_tick();
        assert_eq!(m.working.as_ref().unwrap().stall(), 0.0);
    }

    #[test]
    fn a_silent_model_keeps_its_verb_and_elapsed_time() {
        let mut m = model(120);
        m.working = Some(Working::new());
        assert_eq!(text(&lines(&m)[0]), "· Boondoggling… (0s)");
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
        assert_eq!(drawn, "· Boondoggling… (12s)");
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
