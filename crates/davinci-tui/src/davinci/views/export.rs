//! `4d` — `/export` and `/share`. A session leaving the machine, with a
//! ledger of what goes with it.
//!
//! The screen's job is the second column: what was redacted, and what was kept
//! that names you anyway — absolute paths, branch names, commit subjects. A
//! secret gist is not a private one, and the panel says so rather than
//! implying it with a colour.
//!
//! Mirrors artboard `4d` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::sheet::{facts, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{blank, meter, span, span_on, span_strong, truncate_run, wrap, Surface};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(ledger) = model.export_ledger.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing to export yet — /export writes the session once it holds a turn",
            th.muted,
        )])];
    };

    let formats = Line::from(vec![
        span("format ", th.border),
        span_on(" .html ", th.background, Some(th.primary)),
        span(" ", th.border),
        span(" .jsonl ", th.muted),
        span(" gist ", th.muted),
        span("   one page, no assets, opens offline", th.border),
    ]);

    let mut ledger_rows: Vec<Vec<Span<'static>>> = ledger
        .included
        .iter()
        .map(|text| {
            vec![
                span_strong(format!("{} ", State::Done.glyph()), th.success, th),
                span(text.clone(), th.muted),
            ]
        })
        .collect();
    ledger_rows.push(Vec::new());
    ledger_rows.extend(ledger.excluded.iter().map(|(state, text)| {
        vec![
            span_strong(format!("{} ", state.glyph()), th.state_color(*state), th),
            span(text.clone(), th.muted),
        ]
    }));
    ledger_rows.push(Vec::new());
    let turns = if ledger.turns.is_empty() {
        "every turn".to_string()
    } else {
        format!("{} of {} turns", ledger.turns, ledger.turns)
    };
    let mut written_row = vec![
        span_strong(format!("{} ", State::Done.glyph()), th.success, th),
        span(format!("wrote {turns}"), th.text),
        span("  ", th.border),
    ];
    written_row.extend(meter(1.0, 24, th, Some(th.success)));
    written_row.push(span(
        format!("  {} · {}", ledger.size, ledger.elapsed),
        th.border,
    ));
    ledger_rows.push(written_row);

    let written = Surface::new(width, th)
        .title(vec![span("WHAT LEAVES THE SESSION", th.primary)])
        .rows(ledger_rows)
        .lines();

    let uploaded = if ledger.file.is_empty() {
        "uploaded to your GitHub account".to_string()
    } else {
        format!("uploaded as {} to your GitHub account", ledger.file)
    };
    let mut share_rows: Vec<Vec<Span<'static>>> = vec![
        vec![
            span(format!("{} ", State::Read.glyph()), th.secondary),
            span(uploaded, th.muted),
            span(format!(" · {}", ledger.size), th.border),
        ],
        vec![
            span(format!("{} ", State::Read.glyph()), th.secondary),
            span(ledger.gist.clone(), th.text),
            span(" · copied to the clipboard", th.border),
        ],
    ];
    for row in wrap(
        "! secret is not private — anyone with the link can read the whole session",
        width.saturating_sub(6),
    ) {
        share_rows.push(vec![span(row, th.warning)]);
    }
    share_rows.push(Vec::new());
    share_rows.push(vec![
        span("[o]", th.primary),
        span(" open in browser", th.text),
        span("   [c]", th.primary),
        span(" copy link again", th.muted),
        span("   [d]", th.primary),
        span(" delete the gist", th.muted),
        span("   [esc]", th.border),
        span(" done", th.border),
    ]);

    let share = Surface::new(width, th)
        .border(th.secondary)
        .title(vec![
            span("SHARE", th.secondary),
            span(" · ", th.border),
            span("SECRET GIST", th.muted),
        ])
        .rows(share_rows)
        .lines();

    let tail = vec![
        Line::from(vec![
            span(".jsonl round-trips", th.muted),
            span(" · ", th.border),
            span("/import", th.text),
            span(" resumes it on any machine", th.muted),
        ]),
        Line::from(vec![span(
            "exports are written next to the cwd, never to the session store",
            th.border,
        )]),
    ];

    let mut out = vec![formats, blank()];
    out.extend(written);
    out.push(blank());
    out.extend(share);
    out.push(blank());
    out.extend(tail);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the session leaving in the header,
/// `exporting` in the status bar, the keys inside the share panel rather
/// than a hint row, no composer.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let ledger = model.export_ledger.as_ref();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                ledger
                    .filter(|l| !l.session.is_empty())
                    .map(|l| vec![span(l.session.clone(), th.text)])
                    .unwrap_or_default(),
                ledger
                    .filter(|l| !l.turns.is_empty())
                    .map(|l| vec![span(format!("{} turns", l.turns), th.muted)])
                    .unwrap_or_default(),
                ledger
                    .filter(|l| !l.session_bytes.is_empty())
                    .map(|l| vec![span(format!("{} jsonl", l.session_bytes), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: ledger.map(|_| vec![span("exporting", th.muted)]),
        status_right: None,
        hints: Vec::new(),
        escape: None,
        composer: Composer::Hidden,
        echo: Some(
            ledger
                .filter(|l| !l.file.is_empty())
                .map(|l| format!("/export {}", l.file))
                .unwrap_or_else(|| "/export".into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.export_ledger = Some(fixtures::export_ledger());
        model.toggle_screen(Screen::Export);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_ledger_says_what_leaves_what_is_redacted_and_what_was_written() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows[0].starts_with("format"), "{}", rows[0]);
        assert!(rows
            .iter()
            .any(|row| row.contains("WHAT LEAVES THE SESSION")));
        assert!(rows
            .iter()
            .any(|row| row.contains("× api keys and bearer tokens · redacted")));
        assert!(rows
            .iter()
            .any(|row| row.contains("! absolute paths · kept, they name your machine")));
        assert!(rows
            .iter()
            .any(|row| row.contains("✓ wrote 42 of 42 turns") && row.contains("2.9 MB · 1.4s")));
    }

    #[test]
    fn the_share_panel_names_the_gist_and_warns_that_secret_is_not_private() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("SHARE · SECRET GIST")));
        assert!(rows.iter().any(
            |row| row.contains("uploaded as review-agent-runtime.html to your GitHub account")
        ));
        assert!(rows.iter().any(|row| row.contains("secret is not private")));
        assert!(rows
            .iter()
            .any(|row| row.contains("[o] open in browser") && row.contains("[esc] done")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn nothing_to_export_says_so() {
        let mut m = model(100);
        m.export_ledger = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to export yet")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "4d");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "review-agent-runtime │ 42 turns │ 1.8 MB jsonl");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "exporting");
        assert_eq!(c.escape, None);
        assert_eq!(c.composer, Composer::Hidden);
        assert_eq!(c.echo.as_deref(), Some("/export review-agent-runtime.html"));
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
