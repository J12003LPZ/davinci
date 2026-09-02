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
//! Mirrors artboard `6d` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::{HunkKind, Model, ReviewFile};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, footnote, indent, run_width, selection_bar, span, span_on, span_strong,
    spread, spread_on, truncate_run, SELECTION_BAR,
};

/// The right-aligned count columns and the note column.
const COUNT: usize = 5;
const NOTE: usize = 19;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
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

    let mut out: Vec<Line<'static>> = Vec::new();

    // A wide working tree runs long: show a window around the selection and
    // count the rest.
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
        out.push(row(file, index == selected, width, th));
    }
    if end < total {
        out.push(Line::from(vec![span(
            format!("… {} more below", total - end),
            th.border,
        )]));
    }
    out.push(blank());

    let mut rule_left = vec![
        span_strong(format!("{} ", glyph::DELTA), th.primary, th),
        span(current.path.clone(), th.text),
        span(format!(" {}", plus(current.adds)), th.success),
        span(format!(" {}", minus(current.dels)), th.error),
    ];
    let mut rule_right = Vec::new();
    if !current.hunk_note.is_empty() {
        rule_right.push(span(current.hunk_note.clone(), th.border));
        rule_right.push(span(" · ", th.border));
    }
    rule_right.push(span("j k to move", th.border));
    if run_width(&rule_left) + run_width(&rule_right) + 1 > width {
        rule_left.truncate(2);
    }
    out.push(spread(width, rule_left, rule_right));
    if !current.hunk_header.is_empty() {
        out.push(indent(
            2,
            vec![
                span("│ ", th.border),
                span(current.hunk_header.clone(), th.border),
            ],
        ));
    }
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

    if !review.warning.is_empty() {
        let (warning, aside) = review
            .warning
            .split_once(" · ")
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or_else(|| (review.warning.clone(), String::new()));
        let mut left = vec![
            span_strong(format!("{} ", glyph::ATTENTION), th.warning, th),
            span(warning, th.text),
        ];
        if !aside.is_empty() {
            left.push(span(" · ", th.border));
            left.push(span(aside, th.border));
        }
        out.extend(footnote(
            width,
            left,
            vec![span("revert is per file and per hunk", th.border)],
            th,
        ));
    }
    if !review.tests.is_empty() {
        let (tests, elapsed) = review
            .tests
            .split_once(" · ")
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or_else(|| (review.tests.clone(), String::new()));
        let mut left = vec![
            span_strong(format!("{} ", glyph::DONE), th.success, th),
            span(tests, th.muted),
        ];
        if !elapsed.is_empty() {
            left.push(span(" · ", th.border));
            left.push(span(elapsed, th.border));
        }
        out.extend(footnote(
            width,
            left,
            vec![span(
                "nothing here is committed until you say so",
                th.border,
            )],
            th,
        ));
    }
    // Loose rows — hunk lines included — are cut at the window, never past it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

fn row(file: &ReviewFile, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let tint = selected.then_some(th.surface);
    let deleted = file.state == State::Failed;
    let (base, suffix) = file.path.split_once(" · ").unwrap_or((&file.path, ""));
    let wide = width >= 100;
    let fixed =
        SELECTION_BAR.chars().count() + 2 + COUNT + 1 + COUNT + if wide { 2 + NOTE } else { 0 };
    let path_column = (width as usize).saturating_sub(fixed).saturating_sub(1);
    let suffix_width = if suffix.is_empty() {
        0
    } else {
        suffix.chars().count() + 3
    };
    let base_room = path_column.saturating_sub(suffix_width).max(4);
    let mut left = vec![
        selection_bar(selected, th),
        strong_on(
            format!("{} ", file.state.glyph()),
            th.state_color(file.state),
            tint,
            th,
        ),
        span_on(
            clip_ellipsis(base, base_room as u16),
            if deleted { th.muted } else { th.text },
            tint,
        ),
    ];
    if !suffix.is_empty() {
        left.push(span_on(format!(" · {suffix}"), th.border, tint));
    }
    let mut right = vec![
        span_on(format!("{:>COUNT$}", plus(file.adds)), th.success, tint),
        span_on(format!(" {:>COUNT$}", minus(file.dels)), th.error, tint),
    ];
    if wide {
        right.push(span_on(
            format!("  {:>NOTE$}", clip_ellipsis(&file.tests, NOTE as u16)),
            tests_color(file.test_state, th),
            tint,
        ));
    }
    spread_on(width, left, right, tint)
}

/// An emphasised run on a tinted row — a selected file's glyph.
fn strong_on(
    content: impl Into<String>,
    color: Color,
    background: Option<Color>,
    th: &Theme,
) -> Span<'static> {
    let mut style = ratatui::style::Style::default()
        .fg(color)
        .add_modifier(th.emphasis);
    if let Some(background) = background {
        style = style.bg(background);
    }
    Span::styled(content.into(), style)
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

/// The sheet's frame (design.md §11): the file count, the totals and the
/// branch in the header, `Δn +a -d` in the status bar, the review keys on
/// the hint row and the composer ready for the fix.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let review = model.review.as_ref();
    let totals = |review: &crate::davinci::model::ReviewSheet| {
        vec![
            span(format!("+{}", review.adds), th.success),
            span(format!(" -{}", review.dels), th.error),
        ]
    };
    SheetChrome {
        header_right: facts(
            th,
            vec![
                review
                    .map(|r| vec![span(format!("{} files", r.files.len()), th.muted)])
                    .unwrap_or_default(),
                review.map(totals).unwrap_or_default(),
                review
                    .filter(|r| !r.branch.is_empty())
                    .map(|r| {
                        let mut run = vec![span(r.branch.clone(), th.secondary)];
                        if !r.behind.is_empty() {
                            run.push(span(" · ", th.border));
                            run.push(span(r.behind.clone(), th.border));
                        }
                        run
                    })
                    .unwrap_or_default(),
            ],
        ),
        status_third: review.map(|r| {
            let mut run = vec![span(
                format!("{}{}", glyph::DELTA, r.files.len()),
                th.primary,
            )];
            run.push(span(" ", th.border));
            run.extend(totals(r));
            run
        }),
        status_right: None,
        hints: vec![
            hint(th, "↑↓ file"),
            hint(th, "j k hunk"),
            hint_dim(th, "enter open in codex"),
            hint_dim(th, "u revert hunk"),
            hint_dim(th, "c commit"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("fix the two legacy.rs references, then commit"),
        echo: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::{Hunk, ReviewSheet};
    use crate::davinci::theme::ColorDepth;

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
                    hunk_header: "@@ 214,7 +214,18 @@ impl OpenAiProvider".into(),
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
                    hunk_header: String::new(),
                    hunk: vec![Hunk::new(HunkKind::Del, "pub struct LegacyProvider {")],
                },
            ],
            adds: 145,
            dels: 127,
            branch: "main".into(),
            behind: "3 commits behind".into(),
            warning: "legacy.rs is gone but 2 files still name it · grafo says the build will fail"
                .into(),
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
        assert!(
            header.contains("88 lines removed · j k to move"),
            "{header}"
        );
        assert!(rows.iter().any(|row| row.contains("LegacyProvider")));
        // The other file's hunk stays folded.
        assert!(!rows.iter().any(|row| row.contains("post_stream")));
    }

    #[test]
    fn every_file_states_its_counts_and_what_the_tests_said() {
        let rows: Vec<String> = lines(&model(120)).iter().map(text).collect();
        // The summary moved to the header: the first row is a file.
        assert!(
            rows[0].starts_with("▌  Δ crates\\davinci-ai\\src\\openai.rs"),
            "{}",
            rows[0]
        );
        assert!(
            rows[0]
                .trim_end()
                .ends_with("+64   -19      ✓ 14 tests pass"),
            "{}",
            rows[0]
        );
        // A deleted file adds nothing: the count is an em dash, not a zero.
        let deleted = rows
            .iter()
            .find(|row| row.contains("legacy.rs"))
            .expect("the deleted row");
        assert!(deleted.starts_with("   × "), "{deleted}");
        assert!(deleted.contains("—   -88"), "{deleted}");
        assert!(rows
            .iter()
            .any(|row| row.contains("│ @@ 214,7 +214,18 @@ impl OpenAiProvider")));
        assert!(rows.iter().any(|row| row.starts_with(
            "! legacy.rs is gone but 2 files still name it · grafo says the build will fail"
        ) && row
            .trim_end()
            .ends_with("revert is per file and per hunk")));
        assert!(rows.iter().any(|row| row
            .starts_with("✓ 212 of 212 tests pass on the changed crates · 41.2s")
            && row
                .trim_end()
                .ends_with("nothing here is committed until you say so")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
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

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "6d");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "7 files │ +145 -127 │ main · 3 commits behind");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "Δ7 +145 -127");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(
            c.composer,
            Composer::Prompt("fix the two legacy.rs references, then commit")
        );
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("↑↓ file │ j k hunk │ enter open in codex │ u revert hunk │ c commit"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }
}
