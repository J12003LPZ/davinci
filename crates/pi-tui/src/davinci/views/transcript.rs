//! The transcript is the interface (design.md §1). No bubbles, no timestamps,
//! no decoration in the body: user turns are `> text` in muted, agent turns
//! open with `◆ davinci`, tool calls are one line each, and prose wraps at the
//! measure even when the terminal is wider.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/transcript.ex`.

use ratatui::text::Line;

use super::{markdown, studio};
use crate::davinci::model::{Entry, HunkKind, Model};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, detail_line, failure_line, indent, span, span_strong, tool_line, wrap,
    MEASURE,
};

/// How many rows of live reasoning are shown while it streams.
const THINKING_TAIL: usize = 3;

/// Render a whole transcript, at a width that may be narrower than the window
/// when the Codex sidebar is open.
pub fn lines(model: &Model, entries: &[Entry], width: u16) -> Vec<Line<'static>> {
    entries
        .iter()
        .flat_map(|entry| entry_lines(model, entry, width))
        .collect()
}

/// The last `height` rows of the transcript, rendered from the end.
///
/// Every entry renders independently, so a window's worth can be built by
/// walking backwards until enough rows exist — rendering the whole transcript
/// four times a second grew with the session and was the frame's whole cost.
pub fn tail_lines(
    model: &Model,
    entries: &[Entry],
    width: u16,
    height: usize,
) -> Vec<Line<'static>> {
    let mut chunks: Vec<Vec<Line<'static>>> = Vec::new();
    let mut total = 0usize;
    for entry in entries.iter().rev() {
        let rows = entry_lines(model, entry, width);
        total += rows.len();
        chunks.push(rows);
        if total >= height {
            break;
        }
    }
    let mut out: Vec<Line<'static>> = chunks.into_iter().rev().flatten().collect();
    if out.len() > height {
        out = crate::davinci::ui::tail(out, height);
    }
    out
}

fn entry_lines(model: &Model, entry: &Entry, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    match entry {
        Entry::Gap => vec![blank()],

        Entry::User(text) => vec![Line::from(vec![
            span(format!("{} ", glyph::USER), th.primary),
            span(clip_ellipsis(text, width.saturating_sub(4)), th.muted),
        ])],

        Entry::Agent(name) => vec![Line::from(vec![
            span_strong(format!("{} ", glyph::AGENT), th.primary, th),
            span(name.clone(), th.text),
        ])],

        Entry::Tool {
            state,
            instrument,
            target,
            duration,
            summary,
        } => vec![tool_line(
            width,
            th,
            *state,
            instrument,
            target,
            duration.as_deref(),
            summary.as_deref(),
        )],

        Entry::Detail(text) => vec![detail_line(th, text)],

        Entry::Failure { what, subject } => vec![failure_line(th, what, subject)],

        Entry::Prose(text) => markdown::lines(th, text, MEASURE.min(width.saturating_sub(2))),

        Entry::Thinking {
            text,
            live,
            seconds,
        } => thinking_lines(th, text, *live, *seconds, width),

        Entry::Studio(steps) => studio::lines(model, steps),

        Entry::Delta {
            path,
            adds,
            dels,
            hunks,
        } => {
            let mut rows = vec![Line::from(vec![
                span(format!("{} ", glyph::DELTA), th.primary),
                span(clip_ellipsis(path, width.saturating_sub(20)), th.text),
                span(format!("  +{adds}"), th.success),
                span(format!(" -{dels}"), th.error),
            ])];
            rows.extend(hunks.iter().map(|hunk| hunk_line(th, hunk, width)));
            rows
        }
    }
}

/// Reasoning: while live, a `⟐ reasoning` row and the last few rows of the
/// summary as they arrive; once done, one muted row with how long it took
/// and its first sentence, so the thought is auditable without crowding the
/// answer. Empty reasoning that finished says only how long it took.
fn thinking_lines(
    theme: &Theme,
    text: &str,
    live: bool,
    seconds: u64,
    width: u16,
) -> Vec<Line<'static>> {
    let measure = MEASURE.min(width.saturating_sub(4));
    if live {
        let mut rows = vec![Line::from(vec![
            span(format!("{} ", glyph::COLLAPSED), theme.primary),
            span("reasoning", theme.muted),
        ])];
        let wrapped = wrap(text.trim(), measure);
        let start = wrapped.len().saturating_sub(THINKING_TAIL);
        rows.extend(
            wrapped
                .into_iter()
                .skip(start)
                .map(|row| indent(2, vec![span(row, theme.muted)])),
        );
        return rows;
    }
    let mut label = if seconds == 0 {
        "reasoned".to_string()
    } else {
        format!("reasoned {seconds}s")
    };
    let first = first_sentence(text);
    if !first.is_empty() {
        label.push_str(" · ");
        label.push_str(&first);
    }
    vec![Line::from(vec![
        span(format!("{} ", glyph::COLLAPSED), theme.border),
        span(clip_ellipsis(&label, measure), theme.muted),
    ])]
}

/// The first sentence of a reasoning summary, on one row: up to the first
/// full stop followed by a space or a line end, headings' `**` dropped.
pub fn first_sentence(text: &str) -> String {
    let flat: String = text
        .replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut end = flat.len();
    for (index, ch) in flat.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let next = flat[index + ch.len_utf8()..].chars().next();
            if next.is_none() || next == Some(' ') {
                end = index + ch.len_utf8();
                break;
            }
        }
    }
    flat[..end].to_string()
}

/// Hunks sit behind a single left rule; no line numbers unless asked (§6).
fn hunk_line(theme: &Theme, hunk: &crate::davinci::model::Hunk, width: u16) -> Line<'static> {
    let (sign, color) = match hunk.kind {
        HunkKind::Add => ("+", theme.success),
        HunkKind::Del => ("-", theme.error),
        HunkKind::Context => (" ", theme.muted),
    };
    indent(
        2,
        vec![
            span("│ ", theme.border),
            span(format!("{sign} "), color),
            span(clip_ellipsis(&hunk.text, width.saturating_sub(10)), color),
        ],
    )
}

/// A tool failure expands to at most four indented lines and keeps the exit
/// code (design.md §6, screen `1b`).
pub const MAX_FAILURE_DETAIL_LINES: usize = 4;

/// Trim a failure body to the four lines the design allows.
pub fn failure_detail(body: &[String]) -> Vec<String> {
    body.iter()
        .take(MAX_FAILURE_DETAIL_LINES)
        .cloned()
        .collect()
}

/// The glyph a tool call carries, from what the tool did. Color only reinforces
/// this (design.md §4).
pub fn tool_state(verb: &str, failed: bool, skipped: bool) -> State {
    if failed {
        return State::Failed;
    }
    if skipped {
        return State::Skipped;
    }
    match verb {
        "read" => State::Read,
        "search" | "grep" | "find" => State::Search,
        "edit" | "write" => State::Delta,
        _ => State::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Hunk, Step};
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::run_width;
    use unicode_width::UnicodeWidthStr;

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        )
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn live_reasoning_shows_its_tail_and_collapses_to_one_row_when_done() {
        let m = model(100);
        let long = (1..=12)
            .map(|n| format!("step {n} of the plan"))
            .collect::<Vec<_>>()
            .join(" ");
        let live = lines(&m, &[Entry::thinking(&long, true, 0)], 100);
        assert_eq!(text(&live[0]), "⟐ reasoning");
        assert_eq!(
            live.len(),
            1 + THINKING_TAIL,
            "{:?}",
            live.iter().map(text).collect::<Vec<_>>()
        );
        assert!(text(&live[1]).starts_with("  "));

        let done = lines(
            &m,
            &[Entry::thinking(
                "**Planning** the file read. Then answer.",
                false,
                4,
            )],
            100,
        );
        assert_eq!(done.len(), 1);
        assert_eq!(text(&done[0]), "⟐ reasoned 4s · Planning the file read.");

        let quick = lines(&m, &[Entry::thinking("", false, 0)], 100);
        assert_eq!(text(&quick[0]), "⟐ reasoned");
    }

    #[test]
    fn prose_renders_as_markdown() {
        let m = model(100);
        let rows = lines(&m, &[Entry::prose("# Title\n\n- one\n- two")], 100);
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(texts[0], "Title");
        assert!(
            texts.iter().any(|row| row.starts_with("· one")),
            "{texts:?}"
        );
    }

    #[test]
    fn tail_lines_matches_a_full_render_tailed_at_every_height() {
        // The frame renders only the tail; it must draw exactly what the
        // full render's tail would have drawn.
        let m = model(100);
        let entries = transcript();
        let full = lines(&m, &entries, 100);
        for height in [1usize, 3, 8, 15, 40, 200] {
            let short = tail_lines(&m, &entries, 100, height);
            let expected: Vec<String> = full
                .iter()
                .skip(full.len().saturating_sub(height))
                .map(text)
                .collect();
            let got: Vec<String> = short.iter().map(text).collect();
            assert_eq!(got, expected, "at height {height}");
        }
    }

    /// Screen `1b`, verbatim.
    fn transcript() -> Vec<Entry> {
        vec![
            Entry::user("explain how the agent runtime works"),
            Entry::Gap,
            Entry::agent("davinci"),
            Entry::tool(
                State::Read,
                "instrumenta",
                "read crates\\davinci-agent\\src\\lib.rs",
                None,
            ),
            Entry::tool(
                State::Search,
                "instrumenta",
                "search \"SessionManager\" · 8 matches",
                None,
            ),
            Entry::tool(
                State::Done,
                "manus",
                "cargo check -p davinci-agent",
                Some("1.84s"),
            ),
            Entry::tool(
                State::Failed,
                "manus",
                "cargo test -p davinci-session",
                Some("0.42s"),
            ),
            Entry::detail("error[E0308] mismatched types · store.rs:118"),
            Entry::Gap,
            Entry::Studio(vec![
                Step::new(State::Done, "surveyed workspace", None),
                Step::new(State::Active, "examining session persistence", None),
                Step::new(State::Queued, "verify provider abstraction", None),
            ]),
            Entry::Gap,
            Entry::prose(
                "A request enters davinci-agent as a Turn, is planned, then dispatched to \
                 the provider adapter. Session state is written after every tool call, so \
                 an interrupt never loses the transcript.",
            ),
            Entry::Gap,
            Entry::Delta {
                path: "crates\\davinci-agent\\src\\runtime.rs".into(),
                adds: 31,
                dels: 8,
                hunks: vec![
                    Hunk::new(HunkKind::Add, "pub async fn execute_stream("),
                    Hunk::new(HunkKind::Del, "    self.execute(req).await"),
                ],
            },
        ]
    }

    #[test]
    fn a_user_turn_is_an_echo_with_no_bubble_and_no_timestamp() {
        let m = model(100);
        let rows = lines(&m, &[Entry::user("run the tests")], 100);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "> run the tests");
        assert_eq!(rows[0].spans[1].style.fg, Some(m.theme.muted));
    }

    #[test]
    fn an_agent_turn_opens_with_the_agent_mark() {
        let m = model(100);
        let rows = lines(&m, &[Entry::agent("davinci")], 100);
        assert_eq!(text(&rows[0]), "◆ davinci");
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.primary));
    }

    #[test]
    fn prose_wraps_at_the_measure_however_wide_the_terminal() {
        let m = model(200);
        let entry = Entry::prose(
            "A request enters davinci-agent as a Turn, is planned, then dispatched to the \
             provider adapter. Session state is written after every tool call, so an \
             interrupt never loses the transcript.",
        );
        for width in [80u16, 100, 160, 200] {
            for row in lines(&m, std::slice::from_ref(&entry), width) {
                let drawn = text(&row);
                assert!(
                    UnicodeWidthStr::width(drawn.as_str()) <= MEASURE as usize,
                    "prose exceeded the measure at width {width}: {drawn:?}"
                );
            }
        }
    }

    #[test]
    fn a_tool_call_is_one_line_with_no_box() {
        let m = model(100);
        let rows = lines(
            &m,
            &[Entry::tool(
                State::Done,
                "manus",
                "cargo fmt",
                Some("0.31s"),
            )],
            100,
        );
        assert_eq!(rows.len(), 1);
        let drawn = text(&rows[0]);
        assert!(drawn.starts_with("  ⎿ ✓ cargo fmt"), "{drawn}");
        assert!(drawn.contains("· 0.31s"), "{drawn}");
        assert!(drawn.ends_with("· manus"), "{drawn}");
        assert!(!drawn.contains('╭'));
    }

    #[test]
    fn every_tool_state_reads_without_color() {
        assert_eq!(tool_state("read", false, false).glyph(), "↳");
        assert_eq!(tool_state("search", false, false).glyph(), "⌕");
        assert_eq!(tool_state("edit", false, false).glyph(), "Δ");
        assert_eq!(tool_state("bash", false, false).glyph(), "✓");
        assert_eq!(tool_state("bash", true, false).glyph(), "×");
        assert_eq!(tool_state("bash", false, true).glyph(), "◌");
    }

    #[test]
    fn a_failure_keeps_its_exit_code_in_at_most_four_indented_lines() {
        let m = model(100);
        let body: Vec<String> = (0..9).map(|i| format!("frame {i}")).collect();
        let kept = failure_detail(&body);
        assert_eq!(kept.len(), 4);

        // Tool detail sits two columns under the tool line (design.md §3).
        let rows = lines(
            &m,
            &[Entry::failure(
                "error[E0308]",
                "mismatched types · store.rs:118",
            )],
            100,
        );
        let drawn = text(&rows[0]);
        assert!(drawn.starts_with("    error[E0308]"), "{drawn}");
        assert!(drawn.contains("store.rs:118"));
        assert_eq!(rows[0].spans[1].style.fg, Some(m.theme.error));
    }

    #[test]
    fn a_failure_detail_carries_a_glyph_under_no_color() {
        let mut m = model(100);
        m.theme = Theme::da_vinci(ColorDepth::TrueColor, true);
        let rows = lines(
            &m,
            &[Entry::failure("1 failed", "store::roundtrip_windows_paths")],
            100,
        );
        let drawn = text(&rows[0]);
        assert!(drawn.contains("! 1 failed"), "{drawn}");
    }

    #[test]
    fn a_delta_block_names_its_path_and_its_counts() {
        let m = model(100);
        let rows = lines(&m, &transcript()[13..], 100);
        let head = text(&rows[0]);
        assert!(
            head.starts_with("Δ crates\\davinci-agent\\src\\runtime.rs"),
            "{head}"
        );
        assert!(head.contains("+31 -8"), "{head}");
        assert_eq!(rows[1].spans[1].style.fg, Some(m.theme.border));
        assert!(text(&rows[1]).contains("│ + pub async fn execute_stream("));
        assert!(text(&rows[2]).contains("│ -     self.execute(req).await"));
        assert_eq!(rows[2].spans[3].style.fg, Some(m.theme.error));
    }

    #[test]
    fn hunks_sit_behind_a_single_left_rule_with_no_line_numbers() {
        let m = model(100);
        for row in lines(&m, &transcript()[13..], 100).iter().skip(1) {
            let drawn = text(row);
            assert!(drawn.starts_with("  │ "), "{drawn}");
            assert!(
                !drawn
                    .trim_start_matches("  │ ")
                    .starts_with(char::is_numeric),
                "line numbers appeared: {drawn}"
            );
        }
    }

    #[test]
    fn screen_1b_renders_the_studio_box_and_screen_1g_collapses_it() {
        let wide = model(100);
        let drawn: Vec<String> = lines(&wide, &transcript(), 100).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.starts_with("╭─ STUDIO ─")));

        let narrow = model(80);
        let drawn: Vec<String> = lines(&narrow, &transcript(), 80).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("STUDIO")));
        assert!(drawn.iter().any(|row| row.contains("studying")));
    }

    #[test]
    fn a_blank_row_separates_blocks_and_never_appears_inside_one() {
        let m = model(100);
        let rows = lines(&m, &transcript(), 100);
        let blanks = rows.iter().filter(|row| run_width(&row.spans) == 0).count();
        assert_eq!(
            blanks,
            transcript()
                .iter()
                .filter(|entry| matches!(entry, Entry::Gap))
                .count()
        );
    }

    #[test]
    fn nothing_in_the_body_is_decorated_except_the_studio_box() {
        let m = model(100);
        let boxed: Vec<String> = lines(&m, &transcript(), 100)
            .iter()
            .map(text)
            .filter(|row| row.contains('╭') || row.contains('╰'))
            .collect();
        assert_eq!(boxed.len(), 2, "only Studio is boxed: {boxed:?}");
        assert!(boxed[0].contains("STUDIO"));
    }
}
