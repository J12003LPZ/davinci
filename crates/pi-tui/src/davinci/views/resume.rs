//! `4a` — `/resume`. The Memoria session list at full width: what each session
//! was, where it was, and what resuming it would cost you.
//!
//! The selected row expands one line rather than opening a second panel
//! (design.md §1), and a session whose branch no longer exists says so on the
//! row — the thing you want to know before you resume it, not after.
//!
//! Mirrors artboard `4a` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, status_meter, Composer, SheetChrome};
use crate::davinci::model::{Model, ResumeRow};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, column_header, footnote, pad, run_width, selection_bar, span, span_on,
    spread, truncate_run, Surface,
};

const BRANCH: u16 = 9;
const TURNS: u16 = 5;
const TOKENS: u16 = 7;
const MODEL: u16 = 7;
const TOUCHED: u16 = 7;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let list = &model.resume_sessions;

    if list.is_empty() {
        return vec![Line::from(vec![span(
            "no sessions on disk yet — the first turn creates one",
            th.muted,
        )])];
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
        .row(vec![
            span(format!("{} ", glyph::SEARCH), th.secondary),
            span("filter sessions…", th.muted),
            Span::styled(" ", caret_style),
        ])
        .lines();
    out.push(Line::from(vec![
        span(
            format!(
                "{} of {} shown",
                list.len(),
                model.session_count.max(list.len())
            ),
            th.border,
        ),
        span(" · ", th.border),
        span("sort recent", th.border),
        span(" · ", th.border),
        span("named only off", th.border),
    ]));
    out.push(blank());

    out.extend(column_header(
        width,
        &[
            ("", LEAD - 1, false),
            ("SESSION", 0, false),
            ("BRANCH", BRANCH, false),
            ("TURNS", TURNS, true),
            ("TOKENS", TOKENS, true),
            ("MODEL", MODEL, false),
            ("TOUCHED", TOUCHED, false),
        ],
        th,
    ));

    for (index, session) in list.iter().enumerate() {
        out.push(row(session, index == selected, width, th));
        if index == selected {
            // What resuming it carries, on the tint under the row.
            let mut note = session.note.clone();
            if !session.commit.is_empty() {
                note.push_str(" · ");
                note.push_str(&session.commit);
            }
            let mut spans = vec![pad(LEAD, Some(th.surface))];
            spans.push(span_on(
                clip_ellipsis(&note, width.saturating_sub(LEAD)),
                th.border,
                Some(th.surface),
            ));
            out.push(spread(width, spans, Vec::new()));
        } else if let Some(warning) = &session.warning {
            out.push(Line::from(vec![
                pad(LEAD, None),
                span(
                    clip_ellipsis(warning, width.saturating_sub(LEAD)),
                    th.warning,
                ),
            ]));
        }
    }

    out.push(blank());
    out.push(Line::from(vec![
        span("selected ", th.muted),
        span(current.name.clone(), th.text),
        span(" · last message ", th.muted),
        span(format!("“{}”", clip_ellipsis(&current.last, 38)), th.border),
    ]));
    let mut path = vec![span(current.path.clone(), th.border)];
    if !current.size.is_empty() {
        path.push(span(" · ", th.border));
        path.push(span(current.size.clone(), th.muted));
    }
    out.push(Line::from(path));
    out.extend(footnote(
        width,
        vec![span(
            "resuming replays the transcript, not the tools",
            th.muted,
        )],
        vec![span("f forks instead of continuing", th.border)],
        th,
    ));
    // The loose rows outside the filter box are cut to the window, never
    // wrapped: the sheet reports one height per row and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// `1288490188` → `1.2G`, `8589934592` → `8G`.
pub fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 10.0 || (gb - gb.round()).abs() < 0.05 {
        format!("{}G", gb.round() as u64)
    } else {
        format!("{gb:.1}G")
    }
}

/// The sheet's frame (design.md §11): the store in the header, its size as
/// a meter in the status bar, no composer — the filter box is the input.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let count = model.session_count.max(model.resume_sessions.len());
    let disk = model.facts.sessions_disk.filter(|(_, cap)| *cap > 0);
    SheetChrome {
        header_right: facts(
            th,
            vec![
                vec![span("this project", th.muted)],
                (count > 0)
                    .then(|| vec![span(format!("{count} sessions"), th.muted)])
                    .unwrap_or_default(),
                disk.map(|(used, _)| {
                    vec![span(
                        format!("{} on disk", gigabytes(used).replace('G', " GB")),
                        th.muted,
                    )]
                })
                .unwrap_or_default(),
            ],
        ),
        status_third: (count > 0).then(|| vec![span(format!("{count} sessions"), th.muted)]),
        status_right: disk.map(|(used, cap)| {
            status_meter(
                th,
                "disk",
                used as f64 / cap as f64,
                &gigabytes(used),
                &gigabytes(cap),
            )
        }),
        hints: vec![
            hint(th, "enter resume"),
            hint(th, "f fork"),
            hint(th, "ctrl+r rename"),
            hint(th, "ctrl+s sort"),
            hint_dim(th, "ctrl+p paths"),
            hint_dim(th, "ctrl+d delete"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
    }
}

fn row(session: &ResumeRow, selected: bool, width: u16, th: &Theme) -> Line<'static> {
    let band = selected.then_some(th.surface);
    let state = if selected {
        State::Active
    } else if session.warning.is_some() {
        State::Attention
    } else {
        State::Queued
    };
    let name_color = if selected || session.named {
        th.text
    } else {
        th.muted
    };
    let detail = if session.named { th.muted } else { th.border };

    let fixed = LEAD + BRANCH + 1 + TURNS + 1 + TOKENS + 1 + MODEL + 1 + TOUCHED;
    let name_column = width.saturating_sub(fixed).saturating_sub(1);
    let mut spans = vec![
        selection_bar(selected, th),
        Span::styled(
            format!("{} ", state.glyph()),
            Style::default()
                .fg(th.state_color(state))
                .add_modifier(th.emphasis)
                .bg(band.unwrap_or(th.background)),
        ),
        span_on(
            format!(
                "{:<w$} ",
                clip_ellipsis(&session.name, name_column),
                w = name_column as usize
            ),
            name_color,
            band,
        ),
        span_on(
            format!("{:<w$} ", session.branch, w = BRANCH as usize),
            th.secondary,
            band,
        ),
        span_on(
            format!("{:>w$} ", session.turns, w = TURNS as usize),
            detail,
            band,
        ),
        span_on(
            format!("{:>w$} ", session.tokens, w = TOKENS as usize),
            detail,
            band,
        ),
        span_on(
            format!("{:<w$} ", session.model, w = MODEL as usize),
            detail,
            band,
        ),
        span_on(
            format!("{:<w$}", session.touched, w = TOUCHED as usize),
            detail,
            band,
        ),
    ];
    let gap = width.saturating_sub(run_width(&spans));
    if gap > 0 {
        spans.push(pad(gap, band));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
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
            size: "1.8 MB".into(),
            commit: "a3a6f31".into(),
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
            "3 of 34 shown · sort recent · named only off",
            "review-agent-runtime",
            "f forks instead of continuing",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn the_selected_row_expands_and_names_its_file() {
        let mut m = model(100);
        m.resume_index = 1;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("selected") && row.contains("implement-rpc-mode")));
        assert!(rows
            .iter()
            .any(|row| row.contains("01JB2K") && row.contains("1.8 MB")));
        assert!(rows
            .iter()
            .any(|row| row.contains("forked from provider-parity at turn 12 · a3a6f31")));
        let marked = rows
            .iter()
            .find(|row| row.contains("implement-rpc-mode"))
            .unwrap();
        assert!(marked.starts_with("▌  ◉"), "{marked}");
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
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "4a");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "this project │ 34 sessions │ 1.2 GB on disk");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "34 sessions");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(right.starts_with("disk "), "{right}");
        assert!(right.ends_with(" 1.2G/8G"), "{right}");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("enter resume │ f fork │ ctrl+r rename"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }

    #[test]
    fn without_a_disk_cap_the_meter_is_omitted_not_invented() {
        let mut m = model(100);
        m.facts.sessions_disk = Some((1_000, 0));
        assert!(chrome(&m).status_right.is_none());
        let header: String = chrome(&m)
            .header_right
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(header, "this project │ 34 sessions");
    }

    #[test]
    fn gigabytes_read_short() {
        assert_eq!(gigabytes(1_288_490_188), "1.2G");
        assert_eq!(gigabytes(8_589_934_592), "8G");
    }

    #[test]
    fn no_row_overflows_the_window_and_the_filter_box_is_row_exact() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let rows = lines(&m);
            assert_eq!(run_width(&rows[0].spans), width, "at {width}");
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
