//! `6d` — the Δ review: every file the turn touched, before you keep any of
//! it.
//!
//! The file list carries what the change did to the tests, because "+21 −6"
//! and "no test covers this path" are two different pieces of news. The
//! selected file expands to its hunks behind a single left rule, additions in
//! success, deletions in error, context muted — the Δ block of design.md §6
//! at full size. A review screen that showed one file's diff under another
//! file's name would be worse than showing none.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/diff.ex`.

use ratatui::style::Color;
use ratatui::text::Line;

use crate::davinci::model::{HunkKind, Model, ReviewFile};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{blank, clip_ellipsis, indent, span, span_on, span_strong};

/// Cells the path column takes, and the right-aligned tests column.
const PATH: usize = 44;
const TESTS: usize = 19;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(review) = model
        .review
        .as_ref()
        .filter(|review| !review.files.is_empty())
    else {
        return vec![Line::from(vec![span(
            "there are no changes to review",
            model.theme.muted,
        )])];
    };
    let selected = model.diff_index % review.files.len();
    let current = &review.files[selected];

    let mut out = vec![
        Line::from(vec![
            span(format!("{} files", review.files.len()), th.muted),
            span("   ", th.border),
            span(format!("+{}", review.adds), th.success),
            span(" ", th.border),
            span(format!("-{}", review.dels), th.error),
            span("   ", th.border),
            span(review.branch.clone(), th.secondary),
            span(format!(" · {}", review.behind), th.border),
        ]),
        blank(),
    ];

    // A wide working tree runs long and the sheet is tail-anchored: show a
    // window around the selection and count the rest.
    const WINDOW: usize = 10;
    let total = review.files.len();
    let start = selected
        .saturating_sub(WINDOW / 2)
        .min(total.saturating_sub(WINDOW));
    let end = (start + WINDOW).min(total);
    if start > 0 {
        out.push(Line::from(vec![span(
            format!("… {start} above"),
            th.border,
        )]));
    }
    for (index, file) in review.files.iter().enumerate() {
        if index < start || index >= end {
            continue;
        }
        out.push(row(file, index == selected, model.width, th));
    }
    if end < total {
        out.push(Line::from(vec![span(
            format!("… {} more below", total - end),
            th.border,
        )]));
    }
    out.push(blank());

    out.push(Line::from(vec![
        span_strong(format!("{} ", glyph::DELTA), th.primary, th),
        span(current.path.clone(), th.text),
        span(format!("  {} ", plus(current.adds)), th.success),
        span(minus(current.dels), th.error),
        span(format!("   {} · j k to move", current.hunk_note), th.border),
    ]));
    // Keywords, strings and numbers take their own ink on changed rows;
    // context rows stay quiet, the way the Δ block does (design.md §6).
    let language = super::highlight::language_of(&current.path);
    for hunk in &current.hunk {
        let mut spans = vec![
            span("│ ", th.border),
            span(marker(hunk.kind), marker_color(hunk.kind, th)),
        ];
        let body = body_color(hunk.kind, th);
        if hunk.kind == HunkKind::Context {
            spans.push(span(hunk.text.clone(), body));
        } else {
            spans.extend(super::highlight::spans(th, language, &hunk.text, body));
        }
        out.push(indent(2, spans));
    }
    out.push(blank());

    out.push(Line::from(vec![
        span_strong(format!("{} ", glyph::ATTENTION), th.warning, th),
        span(review.warning.clone(), th.muted),
    ]));
    out.push(Line::from(vec![
        span_strong(format!("{} ", glyph::DONE), th.success, th),
        span(review.tests.clone(), th.muted),
    ]));
    out.push(Line::from(vec![
        span("↑↓ file", th.border),
        span(" · ", th.border),
        span("j k hunk", th.border),
        span(" · ", th.border),
        span("enter open in codex", th.border),
        span(" · ", th.border),
        span("u revert hunk", th.border),
        span(" · ", th.border),
        span("c commit", th.border),
    ]));
    out.push(Line::from(vec![span(
        "nothing here is committed until you say so",
        th.border,
    )]));
    // Loose rows — hunk lines included — are cut at the window, never past it.
    out.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, model.width)))
        .collect()
}

fn row(file: &ReviewFile, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let tint = selected.then_some(th.surface);
    // Below 100 the tests column follows the counts immediately rather than
    // being padded to a fixed grid, so the row never overflows the window.
    let path_column = if width >= 100 { PATH } else { PATH.min(30) };
    let mut spans = vec![
        strong_on(
            format!("{} ", file.state.glyph()),
            th.state_color(file.state),
            tint,
            th,
        ),
        span_on(
            format!(
                "{:<w$}",
                clip_ellipsis(&file.path, (path_column - 2) as u16),
                w = path_column - 1
            ),
            if selected { th.text } else { th.muted },
            tint,
        ),
        span_on(format!("{:>5} ", plus(file.adds)), th.success, tint),
        span_on(format!("{:>5}  ", minus(file.dels)), th.error, tint),
    ];
    if width >= 100 {
        spans.push(span_on(
            format!("{:>TESTS$}", clip_ellipsis(&file.tests, TESTS as u16)),
            tests_color(file.test_state, th),
            tint,
        ));
    }
    Line::from(spans)
}

/// An emphasised run on a tinted row — a selected file's glyph.
fn strong_on(
    content: impl Into<String>,
    color: Color,
    background: Option<Color>,
    th: &Theme,
) -> ratatui::text::Span<'static> {
    let mut style = ratatui::style::Style::default()
        .fg(color)
        .add_modifier(th.emphasis);
    if let Some(background) = background {
        style = style.bg(background);
    }
    ratatui::text::Span::styled(content.into(), style)
}

/// `—` where a count does not apply: a new file deletes nothing, a deleted
/// file adds nothing.
fn plus(count: Option<u32>) -> String {
    count.map(|n| format!("+{n}")).unwrap_or_else(|| "—".into())
}

fn minus(count: Option<u32>) -> String {
    count.map(|n| format!("-{n}")).unwrap_or_else(|| "—".into())
}

fn tests_color(state: State, th: &Theme) -> Color {
    match state {
        State::Done => th.success,
        State::Attention => th.warning,
        _ => th.border,
    }
}

fn marker(kind: HunkKind) -> &'static str {
    match kind {
        HunkKind::Add => "+ ",
        HunkKind::Del => "- ",
        HunkKind::Context => "  ",
    }
}

fn marker_color(kind: HunkKind, th: &Theme) -> Color {
    match kind {
        HunkKind::Add => th.success,
        HunkKind::Del => th.error,
        HunkKind::Context => th.border,
    }
}

fn body_color(kind: HunkKind, th: &Theme) -> Color {
    match kind {
        HunkKind::Add => th.text,
        HunkKind::Del | HunkKind::Context => th.muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Hunk, ReviewSheet};
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::run_width;

    fn sheet() -> ReviewSheet {
        ReviewSheet {
            files: vec![
                ReviewFile {
                    state: State::Delta,
                    path: "crates\\davinci-ai\\src\\openai.rs".into(),
                    adds: Some(64),
                    dels: Some(19),
                    tests: "✓ 14 tests pass".into(),
                    test_state: State::Done,
                    hunk_note: "hunk 2 of 5".into(),
                    hunk: vec![
                        Hunk::new(HunkKind::Context, "pub async fn complete(…) {"),
                        Hunk::new(HunkKind::Del, "    let body = self.post(req).await?;"),
                        Hunk::new(
                            HunkKind::Add,
                            "    let mut stream = self.post_stream(req).await?;",
                        ),
                    ],
                },
                ReviewFile {
                    state: State::Failed,
                    path: "crates\\davinci-ai\\src\\legacy.rs · deleted".into(),
                    adds: None,
                    dels: Some(88),
                    tests: "! 2 references left".into(),
                    test_state: State::Attention,
                    hunk_note: "88 lines removed".into(),
                    hunk: vec![Hunk::new(HunkKind::Del, "pub struct LegacyProvider {")],
                },
            ],
            adds: 145,
            dels: 127,
            branch: "main".into(),
            behind: "3 commits behind".into(),
            warning: "legacy.rs is gone but 2 files still name it · the build will fail".into(),
            tests: "212 of 212 tests pass on the changed crates · 41.2s".into(),
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.review = Some(sheet());
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_selected_file_shows_its_own_hunk_and_never_anothers() {
        let mut m = model(120);
        m.diff_index = 1;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let header = rows
            .iter()
            .find(|row| row.contains("j k to move"))
            .expect("a hunk header");
        assert!(header.contains("legacy.rs"), "{header}");
        assert!(rows.iter().any(|row| row.contains("LegacyProvider")));
        // The other file's hunk stays folded.
        assert!(!rows.iter().any(|row| row.contains("post_stream")));
    }

    #[test]
    fn every_file_states_its_counts_and_what_the_tests_said() {
        let rows: Vec<String> = lines(&model(120)).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("2 files") && row.contains("+145")));
        assert!(rows.iter().any(|row| row.contains("openai.rs")
            && row.contains("+64")
            && row.contains("✓ 14 tests pass")));
        // A deleted file adds nothing: the count is an em dash, not a zero.
        assert!(rows
            .iter()
            .any(|row| row.contains("legacy.rs") && row.contains("—")));
        assert!(rows.iter().any(|row| row.contains("the build will fail")));
        assert!(rows
            .iter()
            .any(|row| row.contains("nothing here is committed")));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_breakpoint() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(run_width(&row.spans) <= width, "overflow at {width}");
            }
        }
    }

    #[test]
    fn an_empty_review_says_so() {
        let mut m = model(100);
        m.review = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no changes to review")));
    }

    #[test]
    fn a_changed_hunk_colours_its_keywords_and_its_add_sign() {
        let mut m = model(120);
        m.diff_index = 0;
        let rows = lines(&m);
        let hunk = rows
            .iter()
            .find(|row| text(row).contains("post_stream"))
            .expect("add hunk");
        assert!(
            hunk.spans
                .iter()
                .any(|span| span.content.contains('+') && span.style.fg == Some(m.theme.success)),
            "{hunk:?}"
        );
        assert!(
            hunk.spans.iter().any(|span| {
                span.content.as_ref() == "let" && span.style.fg == Some(m.theme.secondary)
            }),
            "{hunk:?}"
        );
    }
}
