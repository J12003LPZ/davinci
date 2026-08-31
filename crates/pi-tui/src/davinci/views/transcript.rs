//! The transcript is the interface (design.md §1). No bubbles, no timestamps,
//! no decoration in the body: user turns are `> text` in muted, agent turns
//! open with `◆ davinci`, tool calls are one line each, and prose wraps at the
//! measure even when the terminal is wider.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/transcript.ex`.

use ratatui::text::Line;

use super::studio;
use crate::davinci::model::{Entry, HunkKind, Model};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip, detail_line, indent, span, span_strong, tool_line, wrap, MEASURE,
};

/// Render a whole transcript, at a width that may be narrower than the window
/// when the Codex sidebar is open.
pub fn lines(model: &Model, entries: &[Entry], width: u16) -> Vec<Line<'static>> {
    entries
        .iter()
        .flat_map(|entry| entry_lines(model, entry, width))
        .collect()
}

fn entry_lines(model: &Model, entry: &Entry, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    match entry {
        Entry::Gap => vec![blank()],

        Entry::User(text) => vec![Line::from(vec![
            span(format!("{} ", glyph::USER), th.primary),
            span(clip(text, width.saturating_sub(4)), th.muted),
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
        } => vec![tool_line(
            width,
            th,
            *state,
            instrument,
            target,
            duration.as_deref(),
        )],

        Entry::Detail(text) => vec![detail_line(th, text)],

        Entry::Prose(text) => wrap(text, MEASURE.min(width.saturating_sub(2)))
            .into_iter()
            .map(|row| Line::from(vec![span(row, th.text)]))
            .collect(),

        Entry::Studio(steps) => studio::lines(model, steps),

        Entry::Delta {
            path,
            adds,
            dels,
            hunks,
        } => {
            let mut rows = vec![Line::from(vec![
                span(format!("{} ", glyph::DELTA), th.primary),
                span(clip(path, width.saturating_sub(20)), th.text),
                span(format!("  +{adds}"), th.success),
                span(format!(" -{dels}"), th.error),
            ])];
            rows.extend(hunks.iter().map(|hunk| hunk_line(th, hunk, width)));
            rows
        }
    }
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
            span(clip(&hunk.text, width.saturating_sub(10)), color),
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
        assert!(drawn.starts_with("  ✓ manus · cargo fmt"), "{drawn}");
        assert!(drawn.ends_with("0.31s"), "{drawn}");
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

        let rows = lines(
            &m,
            &[Entry::detail(
                "error[E0308] mismatched types · store.rs:118",
            )],
            100,
        );
        let drawn = text(&rows[0]);
        assert!(drawn.starts_with("      error[E0308]"), "{drawn}");
        assert!(drawn.contains("store.rs:118"));
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
