//! The transcript is the interface (design.md §1). No bubbles, no timestamps,
//! user turns retain the prompt glyph, replies start with a bullet, and tool
//! results hang from an indented elbow. Prose keeps a readable measure.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/transcript.ex`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{markdown, studio};
use crate::davinci::model::{Entry, HunkKind, Model};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, detail_line, failure_line, indent, run_width, span, tool_line,
    truncate_run, wrap, MEASURE,
};

/// How many rows of live reasoning are shown while it streams.
const THINKING_TAIL: usize = 3;

/// Tool and shell result elbow, including the non-breaking spacer.
pub const ELBOW: &str = "  ⎿ \u{a0}";

/// Render a whole transcript, at a width that may be narrower than the window
/// when the Codex sidebar is open.
pub fn lines(model: &Model, entries: &[Entry], width: u16) -> Vec<Line<'static>> {
    rendered_blocks(model, entries, width).into_iter().flatten().collect()
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
    let blocks = rendered_blocks(model, entries, width);
    let mut chunks: Vec<Vec<Line<'static>>> = Vec::new();
    let mut total = 0usize;
    for rows in blocks.into_iter().rev() {
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


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Explore {
    Read,
    Search,
    List,
    Shell,
}

fn explore_kind(instrument: &str, target: &str) -> Option<Explore> {
    if instrument == "manus" {
        return Some(Explore::Shell);
    }
    match target.split_once(' ').map_or(target, |(verb, _)| verb) {
        "read" => Some(Explore::Read),
        "search" | "grep" | "find" => Some(Explore::Search),
        "list" | "ls" => Some(Explore::List),
        _ => None,
    }
}

fn rendered_blocks(model: &Model, entries: &[Entry], width: u16) -> Vec<Vec<Line<'static>>> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < entries.len() {
        if !model.show_tool_output {
            let mut calls: Vec<(Explore, &str, bool)> = Vec::new();
            let mut cursor = index;
            let mut consumed = index;
            while cursor < entries.len() {
                match &entries[cursor] {
                    Entry::Gap if !calls.is_empty() => {
                        cursor += 1;
                        consumed = cursor;
                    }
                    Entry::Tool { state, instrument, target, duration, .. }
                        if !matches!(state, State::Failed | State::Attention) =>
                    {
                        if let Some(kind) = explore_kind(instrument, target) {
                            calls.push((kind, target.as_str(), duration.is_some()));
                            cursor += 1;
                            consumed = cursor;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if !calls.is_empty() {
                out.push(group_rows(model, &calls, width));
                index = consumed;
                continue;
            }
        }
        out.push(entry_lines(model, &entries[index], width));
        index += 1;
    }
    out
}

fn group_rows(model: &Model, calls: &[(Explore, &str, bool)], width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let running = model.running && calls.iter().any(|(_, _, done)| !done);
    let mut order: Vec<Explore> = Vec::new();
    for (kind, _, _) in calls {
        if !order.contains(kind) {
            order.push(*kind);
        }
    }
    let clause = |kind: Explore, n: usize| -> String {
        let noun = |one: &str, many: &str| if n == 1 { one } else { many };
        match (kind, running) {
            (Explore::Read, false) => format!("read {n} {}", noun("file", "files")),
            (Explore::Read, true) => format!("reading {n} {}", noun("file", "files")),
            (Explore::Search, false) => format!("searched for {n} {}", noun("pattern", "patterns")),
            (Explore::Search, true) => format!("searching for {n} {}", noun("pattern", "patterns")),
            (Explore::List, false) => format!("listed {n} {}", noun("directory", "directories")),
            (Explore::List, true) => format!("listing {n} {}", noun("directory", "directories")),
            (Explore::Shell, false) => format!("ran {n} shell {}", noun("command", "commands")),
            (Explore::Shell, true) => format!("running {n} shell {}", noun("command", "commands")),
        }
    };
    let mut sentence = order
        .iter()
        .map(|kind| clause(*kind, calls.iter().filter(|(candidate, _, _)| candidate == kind).count()))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(first) = sentence.get(0..1) {
        sentence = format!("{}{}", first.to_uppercase(), &sentence[1..]);
    }
    if !running {
        let mut spans = vec![span("  ", cc.inactive)];
        spans.extend(bold_numbers(&sentence, cc.inactive));
        return vec![Line::from(truncate_run(spans, width))];
    }
    let bullet = if model.tick % 2 == 1 { "  " } else { "● " };
    let mut spans = vec![span(bullet, cc.inactive)];
    spans.extend(bold_numbers(&format!("{sentence}…"), th.text));
    let (kind, target, _) = calls.last().expect("a group has a call");
    let newest = match kind {
        Explore::Shell => format!("$ {target}"),
        _ => target.split_once(' ').map_or(*target, |(_, rest)| rest).to_string(),
    };
    vec![
        Line::from(truncate_run(spans, width)),
        Line::from(truncate_run(
            vec![span(
                format!("{ELBOW}{}", clip_ellipsis(&newest, width.saturating_sub(5))),
                cc.inactive,
            )],
            width,
        )),
    ]
}

fn entry_lines(model: &Model, entry: &Entry, width: u16) -> Vec<Line<'static>> {
    let th = &model.theme;
    match entry {
        Entry::Gap => vec![blank()],

        Entry::User(text) => user_lines(th, text, width),

        Entry::Shell { command, output, .. } => shell_lines(th, command, output, width),

        // The agent's turn is not announced: the reply follows the prompt
        // after a gap, as in claude code. The entry stays in the transcript
        // as the anchor the streaming prose is appended after.
        Entry::Agent(_) => Vec::new(),

        Entry::Tool {
            state,
            instrument,
            target,
            duration,
            summary,
            output,
        } => {
            let mut rows = vec![tool_line(
                width,
                th,
                *state,
                instrument,
                target,
                duration.as_deref(),
                model.tick,
                model.running,
            )];
            if let Some(summary) = summary.as_deref().filter(|s| !s.is_empty()) {
                rows.push(tool_result(th, summary, width));
            } else if !model.show_tool_output && *state != State::Failed {
                if let Some(first) = output.first() {
                    rows.push(tool_result(th, first, width));
                }
            }
            rows.extend(output_rows(
                th,
                *state,
                output,
                model.show_tool_output,
                width,
            ));
            rows
        }

        Entry::Detail(text) => vec![detail_line(th, text)],

        Entry::Failure { what, subject } => vec![failure_line(th, what, subject)],

        Entry::Prose(text) => markdown::lines(th, text, MEASURE.min(width.saturating_sub(2)))
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let mut spans = vec![span(if index == 0 { "● " } else { "  " }, th.text)];
                spans.extend(row.spans);
                Line::from(truncate_run(spans, width))
            })
            .collect(),

        Entry::Thinking {
            text,
            live,
            seconds,
        } => {
            if model.show_tool_output {
                thinking_lines(th, text, *live, *seconds, width)
            } else {
                Vec::new()
            }
        },

        Entry::Studio(steps) => studio::lines(model, steps),

        Entry::Delta {
            path,
            adds,
            dels,
            hunks,
        } => {
            let mut rows = vec![tool_result(th, &change_summary(*adds, *dels), width)];
            let cap = if model.show_tool_output { DELTA_ROWS_EXPANDED } else { DELTA_ROWS_COLLAPSED };
            let language = super::highlight::language_of(path);
            let number_width = hunks
                .iter()
                .filter_map(|hunk| hunk.line)
                .max()
                .map_or(1, |line| line.to_string().len());
            rows.extend(
                hunks.iter().take(cap)
                    .map(|hunk| hunk_line(th, language, hunk, number_width, width)),
            );
            if hunks.len() > cap {
                rows.push(Line::from(span(
                    format!("      … {} more rows", hunks.len() - cap),
                    th.cc().inactive,
                )));
            }
            rows
        }

        Entry::Done { verb, seconds } => {
            let cc = th.cc();
            vec![Line::from(vec![
                span("✻ ", cc.inactive),
                span(format!("{verb} for {}", duration_words(*seconds)), cc.inactive),
            ])]
        }
    }
}

fn tool_result(theme: &Theme, text: &str, width: u16) -> Line<'static> {
    let cc = theme.cc();
    let mut spans = vec![span(ELBOW, cc.inactive)];
    spans.extend(bold_numbers(&clip_ellipsis(text, width.saturating_sub(5)), theme.text));
    Line::from(truncate_run(spans, width))
}


fn user_lines(theme: &Theme, text: &str, width: u16) -> Vec<Line<'static>> {
    let cc = theme.cc();
    let plain = text.replace('`', "");
    let code = code_ranges(text);
    let wrapped = crate::wrap_text_with_ansi(&plain, width.saturating_sub(3).max(1) as usize);
    let mut offset = 0usize;
    wrapped.into_iter().enumerate().map(|(row, content)| {
        let lead = if row == 0 { "❯ " } else { "  " };
        let mut spans = vec![Span::styled(
            lead,
            Style::default().fg(if row == 0 { cc.subtle } else { cc.text }).bg(cc.user_bg),
        )];
        for (index, ch) in content.chars().enumerate() {
            let at = offset + index;
            let foreground = if code.iter().any(|(start, end)| at >= *start && at < *end) {
                cc.permission
            } else {
                cc.text
            };
            spans.push(Span::styled(ch.to_string(), Style::default().fg(foreground).bg(cc.user_bg)));
        }
        offset += content.chars().count() + 1;
        spans.push(Span::styled(" ", Style::default().bg(cc.user_bg)));
        Line::from(truncate_run(merge_same_style(spans), width))
    }).collect()
}

fn code_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut plain_index = 0;
    let mut open = None;
    for ch in text.chars() {
        if ch == '`' {
            match open.take() {
                Some(start) => ranges.push((start, plain_index)),
                None => open = Some(plain_index),
            }
        } else {
            plain_index += 1;
        }
    }
    ranges
}

fn merge_same_style(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for item in spans {
        match out.last_mut() {
            Some(last) if last.style == item.style => {
                last.content = format!("{}{}", last.content, item.content).into();
            }
            _ => out.push(item),
        }
    }
    out
}

fn shell_lines(theme: &Theme, command: &str, output: &[String], width: u16) -> Vec<Line<'static>> {
    let cc = theme.cc();
    let mut rows = vec![Line::from(truncate_run(
        vec![
            Span::styled("! ", Style::default().fg(cc.bash).bg(cc.bash_bg)),
            Span::styled(format!("{command} "), Style::default().fg(cc.text).bg(cc.bash_bg)),
        ],
        width,
    ))];
    if output.is_empty() {
        rows.push(Line::from(span(format!("{ELBOW}(No output)"), cc.inactive)));
        return rows;
    }
    for (index, row) in output.iter().enumerate() {
        let lead = if index == 0 { ELBOW } else { "     " };
        rows.push(Line::from(truncate_run(
            vec![span(lead, cc.inactive), span(row.clone(), theme.text)],
            width,
        )));
    }
    rows
}

fn bold_numbers(text: &str, color: Color) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut digits = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() != digits && !current.is_empty() {
            let mut piece = span(std::mem::take(&mut current), color);
            if digits { piece.style = piece.style.add_modifier(Modifier::BOLD); }
            out.push(piece);
        }
        digits = ch.is_ascii_digit();
        current.push(ch);
    }
    if !current.is_empty() {
        let mut piece = span(current, color);
        if digits { piece.style = piece.style.add_modifier(Modifier::BOLD); }
        out.push(piece);
    }
    out
}

fn change_summary(adds: u32, dels: u32) -> String {
    let lines = |n| if n == 1 { "line" } else { "lines" };
    match (adds, dels) {
        (0, 0) => "No changes".into(),
        (adds, 0) => format!("Added {adds} {}", lines(adds)),
        (0, dels) => format!("Removed {dels} {}", lines(dels)),
        (adds, dels) => format!("Added {adds} {}, removed {dels} {}", lines(adds), lines(dels)),
    }
}

fn duration_words(seconds: u64) -> String {
    match seconds {
        seconds if seconds < 60 => format!("{seconds}s"),
        seconds if seconds < 3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        seconds => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// Rows of a tool's result shown under its line when expanded (`ctrl+t`).
pub const TOOL_OUTPUT_ROWS: usize = 12;
/// Hunk rows of a live Δ block, collapsed and expanded.
pub const DELTA_ROWS_COLLAPSED: usize = 8;
pub const DELTA_ROWS_EXPANDED: usize = 40;

/// What follows a tool line: a failure's first four rows always (design.md
/// §6), twelve rows of any call when the transcript is expanded, and one
/// row counting what is not shown. Collapsed and successful, nothing.
fn output_rows(
    theme: &Theme,
    state: State,
    output: &[String],
    expanded: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let shown = if expanded {
        TOOL_OUTPUT_ROWS
    } else if state == State::Failed {
        MAX_FAILURE_DETAIL_LINES
    } else {
        0
    };
    if shown == 0 || output.is_empty() {
        return Vec::new();
    }
    let room = width.saturating_sub(6);
    let mut rows: Vec<Line<'static>> = output
        .iter()
        .take(shown)
        .map(|row| detail_line(theme, &clip_ellipsis(row, room)))
        .collect();
    if output.len() > shown {
        rows.push(indent(
            4,
            vec![span(
                format!("… {} more lines", output.len() - shown),
                theme.border,
            )],
        ));
    }
    rows
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
/// On a changed row the sign and plain text keep the row's colour while
/// keywords, strings, comments and numbers take theirs; a context row stays
/// wholly muted.
fn hunk_line(
    theme: &Theme,
    language: Option<super::highlight::Lang>,
    hunk: &crate::davinci::model::Hunk,
    number_width: usize,
    width: u16,
) -> Line<'static> {
    let cc = theme.cc();
    let number = hunk.line.map_or(String::new(), |line| line.to_string());
    let gutter = format!(" {number:>number_width$} ");
    let (sign, sign_color, background) = match hunk.kind {
        HunkKind::Add => ("+", cc.diff_add, Some(cc.diff_add_bg)),
        HunkKind::Del => ("-", cc.diff_del, Some(cc.diff_del_bg)),
        HunkKind::Context => (" ", cc.diff_text, None),
    };
    let paint = |foreground: Color| {
        let style = Style::default().fg(foreground);
        match background {
            Some(background) => style.bg(background),
            None => style,
        }
    };
    let mut gutter_style = paint(if background.is_some() { sign_color } else { cc.diff_text });
    if background.is_none() {
        gutter_style = gutter_style.add_modifier(Modifier::DIM);
    }
    let room = width.saturating_sub(5 + gutter.chars().count() as u16 + 1);
    let text = clip_ellipsis(&hunk.text, room);
    let mut spans = vec![
        Span::raw("     "),
        Span::styled(gutter, gutter_style),
        Span::styled(sign.to_string(), paint(sign_color)),
    ];
    for piece in super::highlight::spans(theme, language, &text, cc.diff_text) {
        let foreground = piece.style.fg.unwrap_or(cc.diff_text);
        spans.push(Span::styled(piece.content.to_string(), paint(foreground)));
    }
    let used = run_width(&spans);
    if let Some(background) = background {
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used) as usize),
            Style::default().bg(background),
        ));
    }
    Line::from(spans)
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
        assert_eq!(texts[0], "● Title");
        assert!(
            texts
                .iter()
                .any(|row| row.trim_start().starts_with("· one")),
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
    fn a_user_turn_is_a_shaded_echo_with_no_timestamp() {
        let m = model(100);
        let rows = lines(&m, &[Entry::user("run the tests")], 100);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "❯ run the tests");
        assert!(rows[0].style.bg.is_none());
        assert_eq!(rows[0].spans[0].style.fg, Some(m.theme.text));
    }

    #[test]
    fn an_agent_turn_has_no_label_line() {
        let m = model(100);
        let rows = lines(
            &m,
            &[
                Entry::user("hello"),
                Entry::Gap,
                Entry::agent("davinci"),
                Entry::prose("Hello! How can I help?"),
            ],
            100,
        );
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert!(
            !texts.iter().any(|row| row.contains("davinci")),
            "{texts:?}"
        );
        assert_eq!(texts[0], "❯ hello");
        assert_eq!(texts[2], "● Hello! How can I help?");
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
                    UnicodeWidthStr::width(drawn.as_str()) <= MEASURE as usize + 2,
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
        assert!(drawn.starts_with("● Shell(cargo fmt)"), "{drawn}");
        assert!(drawn.contains("· 0.31s"), "{drawn}");
        assert!(!drawn.contains("manus"), "{drawn}");
        assert!(!drawn.contains('╭'));
    }

    #[test]
    fn collapsed_success_shows_only_the_first_output_row() {
        let m = model(100);
        let entry = Entry::tool(State::Done, "manus", "cargo fmt", Some("0.31s"))
            .with_output("ok\nfmt done");
        let rows = lines(&m, &[entry], 100);
        assert_eq!(rows.len(), 2);
        assert_eq!(text(&rows[1]), "  ⎿ ok");
    }

    #[test]
    fn expanded_output_caps_at_twelve_and_counts_the_rest() {
        let mut m = model(100);
        m.show_tool_output = true;
        let body: String = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let entry =
            Entry::tool(State::Done, "manus", "cargo test", Some("1.00s")).with_output(&body);
        let rows = lines(&m, &[entry], 100);
        assert_eq!(rows.len(), 1 + TOOL_OUTPUT_ROWS + 1);
        let last = text(&rows[rows.len() - 1]);
        assert!(last.contains("… 8 more lines"), "{last}");
        assert!(text(&rows[1]).contains("line 0"));
    }

    #[test]
    fn a_failure_shows_four_output_rows_and_counts_the_rest() {
        let m = model(100);
        let body: String = (0..9)
            .map(|i| format!("frame {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let entry = Entry::tool(State::Failed, "manus", "cargo test", None).with_output(&body);
        let rows = lines(&m, &[entry], 100);
        assert_eq!(rows.len(), 1 + MAX_FAILURE_DETAIL_LINES + 1);
        assert!(text(&rows[1]).contains("frame 0"));
        let last = text(&rows[rows.len() - 1]);
        assert!(last.contains("… 5 more lines"), "{last}");
    }

    #[test]
    fn a_live_delta_caps_at_eight_collapsed_and_forty_expanded() {
        let hunks: Vec<Hunk> = (0..10)
            .map(|i| Hunk::new(HunkKind::Add, &format!("line {i}")))
            .collect();
        let entry = Entry::Delta {
            path: "src/lib.rs".into(),
            adds: 10,
            dels: 0,
            hunks,
        };
        let m = model(100);
        let collapsed = lines(&m, std::slice::from_ref(&entry), 100);
        assert_eq!(collapsed.len(), 1 + DELTA_ROWS_COLLAPSED + 1);
        let more = text(&collapsed[collapsed.len() - 1]);
        assert!(more.contains("… 2 more rows"), "{more}");

        let mut expanded_model = model(100);
        expanded_model.show_tool_output = true;
        let expanded = lines(&expanded_model, &[entry], 100);
        assert_eq!(expanded.len(), 1 + 10);
    }

    #[test]
    fn a_delta_hunk_colours_keywords_and_the_add_sign() {
        let m = model(100);
        let entry = Entry::Delta {
            path: "src/lib.rs".into(),
            adds: 1,
            dels: 0,
            hunks: vec![Hunk::new(HunkKind::Add, "pub fn foo() {")],
        };
        let rows = lines(&m, &[entry], 100);
        let hunk = &rows[1];
        assert!(
            hunk.spans
                .iter()
                .any(|span| span.content.contains('+') && span.style.fg == Some(m.theme.success)),
            "{hunk:?}"
        );
        assert!(
            hunk.spans
                .iter()
                .any(|span| span.content.as_ref() == "fn"
                    && span.style.fg == Some(m.theme.secondary)),
            "{hunk:?}"
        );
    }

    #[test]
    fn tool_glyphs_survive_no_color() {
        let mut m = model(100);
        m.theme = Theme::da_vinci(ColorDepth::TrueColor, true);
        let rows = lines(
            &m,
            &[
                Entry::tool(State::Done, "manus", "cargo fmt", Some("0.31s")),
                Entry::tool(State::Failed, "manus", "cargo test", None),
                Entry::Delta {
                    path: "src/lib.rs".into(),
                    adds: 1,
                    dels: 0,
                    hunks: vec![Hunk::new(HunkKind::Add, "x")],
                },
            ],
            100,
        );
        let drawn: String = rows.iter().map(text).collect();
        assert!(drawn.contains('✓'), "{drawn}");
        assert!(drawn.contains('×'), "{drawn}");
        assert!(drawn.contains('Δ'), "{drawn}");
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
        assert!(drawn.iter().any(|row| row.starts_with("  Tasks ·")));

        let narrow = model(80);
        let drawn: Vec<String> = lines(&narrow, &transcript(), 80).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("STUDIO")));
        assert!(drawn.iter().any(|row| row.contains("Tasks ·")));
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
        assert!(
            boxed.is_empty(),
            "task checklists must not box the conversation: {boxed:?}"
        );
    }
}
