//! `4a` — `/resume`. The Memoria session list at full width: what each session
//! was, where it was, and what resuming it would cost you.
//!
//! The selected row expands one line rather than opening a second panel
//! (design.md §1), and a session whose branch no longer exists says so on the
//! row — the thing you want to know before you resume it, not after.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/resume.ex`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, ResumeRow};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, indent, span, span_on, truncate_run, wrap, Surface, MEASURE,
};

const NAME: usize = 24;
const BRANCH: usize = 9;
const TURNS: usize = 5;
const TOKENS: usize = 7;
const MODEL: usize = 7;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let list = &model.resume_sessions;

    if list.is_empty() {
        return vec![
            Line::from(vec![span(
                "no sessions on disk yet — the first turn creates one",
                th.muted,
            )]),
            Line::from(vec![span("esc close", th.border)]),
        ];
    }
    let selected = model.resume_index % list.len();
    let current = &list[selected];

    // The filter box the sheet opens with, caret and all.
    let caret_style = if model.blink() {
        Style::default().bg(th.secondary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    let mut out = Surface::new(width, th)
        .border(th.secondary)
        .right(vec![
            span(
                format!("{} of {}", list.len(), model.session_count.max(list.len())),
                th.border,
            ),
            span(" · ", th.border),
            span("sort recent", th.muted),
        ])
        .row(vec![
            span(format!("{} ", glyph::SEARCH), th.secondary),
            span("filter sessions…", th.muted),
            Span::styled(" ", caret_style),
        ])
        .lines();
    out.push(blank());

    out.push(indent(
        2,
        vec![
            span(format!("{:<NAME$}", "SESSION"), th.border),
            span(format!("{:<w$}", "BRANCH", w = BRANCH + 1), th.border),
            span(format!("{:>TURNS$} ", "TURNS"), th.border),
            span(format!("{:>TOKENS$} ", "TOKENS"), th.border),
            span(format!("{:<w$}", "MODEL", w = MODEL + 1), th.border),
            span("TOUCHED", th.border),
        ],
    ));

    // The sheet is tail-anchored; a long store would push its head off
    // screen, so a window follows the selection and the rest is counted.
    const WINDOW: usize = 12;
    let total = list.len();
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
    for (index, session) in list.iter().enumerate() {
        if index < start || index >= end {
            continue;
        }
        out.push(row(session, index == selected, th));
        if index == selected {
            out.push(note(&session.note, th.border, th));
        } else if let Some(warning) = &session.warning {
            out.push(note(warning, th.warning, th));
        }
    }
    if end < total {
        out.push(Line::from(vec![span(
            format!("… {} more below", total - end),
            th.border,
        )]));
    }

    out.push(blank());
    out.push(Line::from(vec![
        span("selected ", th.muted),
        span(current.name.clone(), th.text),
        span(" · last message ", th.muted),
        span(format!("“{}”", clip_ellipsis(&current.last, 38)), th.border),
    ]));
    out.push(Line::from(vec![span(current.path.clone(), th.border)]));
    for row in wrap(
        "resuming replays the transcript, not the tools — nothing runs until \
         you send the next turn",
        MEASURE,
    ) {
        out.push(Line::from(vec![span(row, th.muted)]));
    }
    out.push(Line::from(vec![
        span("enter resume", th.border),
        span(" · ", th.border),
        span("f fork", th.border),
        span(" · ", th.border),
        span("ctrl+r rename", th.border),
        span(" · ", th.border),
        span("ctrl+s sort", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // The loose rows outside the filter box are cut to the window, never
    // wrapped: the sheet reports one height per row and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

fn row(session: &ResumeRow, selected: bool, th: &Theme) -> Line<'static> {
    let band = selected.then_some(th.surface);
    let state = if selected {
        State::Active
    } else if session.warning.is_some() {
        State::Attention
    } else {
        State::Queued
    };
    let name_color = if selected {
        th.text
    } else if session.named {
        th.muted
    } else {
        th.border
    };
    let detail = if session.named { th.muted } else { th.border };

    Line::from(vec![
        Span::styled(
            format!("{} ", state.glyph()),
            Style::default()
                .fg(th.state_color(state))
                .add_modifier(th.emphasis)
                .bg(band.unwrap_or(th.background)),
        ),
        span_on(
            format!(
                "{:<w$}",
                clip_ellipsis(&session.name, (NAME - 2) as u16),
                w = NAME - 2
            ),
            name_color,
            band,
        ),
        span_on(
            format!("{:<w$}", session.branch, w = BRANCH + 1),
            th.secondary,
            band,
        ),
        span_on(format!("{:>TURNS$} ", session.turns), detail, band),
        span_on(format!("{:>TOKENS$} ", session.tokens), detail, band),
        span_on(
            format!("{:<w$}", session.model, w = MODEL + 1),
            detail,
            band,
        ),
        span_on(session.touched.clone(), detail, band),
    ])
}

fn note(text: &str, color: ratatui::style::Color, _th: &Theme) -> Line<'static> {
    indent(2, vec![span(clip_ellipsis(text, MEASURE), color)])
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn session(name: &str, named: bool, warning: Option<&str>) -> ResumeRow {
        ResumeRow {
            name: name.into(),
            branch: "main".into(),
            turns: "42".into(),
            tokens: "128k".into(),
            model: "sonnet".into(),
            touched: "3m".into(),
            named,
            warning: warning.map(str::to_string),
            note: "forked from provider-parity at turn 12".into(),
            last: "now fix the store.rs type error".into(),
            path: "~\\.pi\\agent\\sessions\\--dev--x\\01JB2K….jsonl".into(),
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.resume_sessions = vec![
            session("review-agent-runtime", true, None),
            session("implement-rpc-mode", true, None),
            session(
                "fix-git-hooks",
                true,
                Some("! branch hooks no longer exists · resuming replays against main"),
            ),
        ];
        model.session_count = 34;
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_list_carries_its_columns_and_its_count() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        for expected in [
            "SESSION",
            "BRANCH",
            "TURNS",
            "TOKENS",
            "MODEL",
            "TOUCHED",
            "3 of 34",
            "review-agent-runtime",
            "enter resume",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
    }

    #[test]
    fn the_selected_row_expands_and_names_its_file() {
        let mut m = model(100);
        m.resume_index = 1;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("selected") && row.contains("implement-rpc-mode")));
        assert!(rows.iter().any(|row| row.contains("01JB2K")));
        assert!(rows
            .iter()
            .any(|row| row.contains("forked from provider-parity")));
    }

    #[test]
    fn a_dead_branch_is_warned_about_on_the_row() {
        let m = model(100);
        let rows = lines(&m);
        let warned = rows
            .iter()
            .find(|row| text(row).contains("branch hooks no longer exists"))
            .expect("the warning is drawn");
        assert_eq!(warned.spans[1].style.fg, Some(m.theme.warning));
        let marked = rows
            .iter()
            .find(|row| text(row).contains("fix-git-hooks"))
            .expect("the warned row");
        assert!(text(marked).contains(glyph::ATTENTION));
    }

    #[test]
    fn an_empty_store_says_so_rather_than_a_bare_table() {
        let mut m = model(100);
        m.resume_sessions.clear();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no sessions on disk")));
    }

    #[test]
    fn no_row_overflows_the_window_and_the_filter_box_is_row_exact() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let sheet = width.min(MEASURE + 14);
            let rows = lines(&m);
            assert_eq!(run_width(&rows[0].spans), sheet, "at {width}");
            for row in &rows {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(row)
                );
            }
        }
    }
}
