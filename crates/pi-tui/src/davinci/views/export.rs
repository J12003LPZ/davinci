//! `4d` — `/export` and `/share`. A session leaving the machine, with a
//! ledger of what goes with it.
//!
//! The screen's job is the second column: what was redacted, and what was kept
//! that names you anyway — absolute paths, branch names, commit subjects. A
//! secret gist is not a private one, and the panel says so rather than
//! implying it with a colour.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/export.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State};
use crate::davinci::ui::{
    blank, meter, span, span_on, span_strong, truncate_run, wrap, Surface, MEASURE,
};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(ledger) = model.export_ledger.as_ref() else {
        return vec![Line::from(vec![span(
            "nothing to export yet — /export writes the session once it holds a turn",
            th.muted,
        )])];
    };

    let echo = Line::from(vec![
        span(format!("{} ", glyph::USER), th.primary),
        span("/export", th.muted),
    ]);

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
    let mut written_row = vec![
        span_strong(format!("{} ", State::Done.glyph()), th.success, th),
        span("written in full", th.text),
        span("  ", th.border),
    ];
    written_row.extend(meter(1.0, 20, th, Some(th.success)));
    written_row.push(span(format!("  {}", ledger.elapsed), th.border));
    ledger_rows.push(written_row);

    let written = Surface::new(width, th)
        .title(vec![span("WHAT LEAVES THE SESSION", th.primary)])
        .right(vec![span(ledger.size.clone(), th.border)])
        .rows(ledger_rows)
        .lines();

    let mut share_rows: Vec<Vec<Span<'static>>> = vec![
        vec![
            span(format!("{} ", State::Read.glyph()), th.secondary),
            span("uploaded to your GitHub account", th.muted),
            span(format!("  {}", ledger.size), th.border),
        ],
        vec![
            span(format!("{} ", State::Read.glyph()), th.secondary),
            span(ledger.gist.clone(), th.text),
            span("  copied to the clipboard", th.border),
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
            "exports are written next to the cwd, never into the session store",
            th.border,
        )]),
    ];

    let mut out = vec![echo, blank(), formats, blank()];
    out.extend(written);
    out.push(blank());
    out.extend(share);
    out.push(blank());
    out.extend(tail);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::ExportLedger;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.export_ledger = Some(ExportLedger {
            included: vec![
                "42 turns of prose and thinking".into(),
                "31 tool calls with their output".into(),
            ],
            excluded: vec![
                (
                    State::Failed,
                    "api keys and bearer tokens · redacted".into(),
                ),
                (
                    State::Attention,
                    "absolute paths · kept, they name your machine".into(),
                ),
            ],
            size: "2.9 MB".into(),
            elapsed: "1.4s".into(),
            gist: "https://gist.github.com/9f21c4…a70".into(),
        });
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_ledger_states_what_leaves_and_what_was_redacted() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("WHAT LEAVES THE SESSION")));
        assert!(rows.iter().any(|row| row.contains("42 turns of prose")));
        assert!(rows
            .iter()
            .any(|row| row.contains("api keys and bearer tokens")));
        assert!(rows.iter().any(|row| row.contains("2.9 MB")));
    }

    #[test]
    fn a_secret_gist_is_named_and_warned_about() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("https://gist.github.com/9f21c4…a70")));
        assert!(rows.iter().any(|row| row.contains("secret is not private")));
    }

    #[test]
    fn with_nothing_to_export_the_screen_says_so() {
        let mut m = model(100);
        m.export_ledger = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("nothing to export yet"));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
