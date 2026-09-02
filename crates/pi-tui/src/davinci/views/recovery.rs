//! `6c` — what ctrl+c actually did, and what the provider did before it.
//!
//! This is a transcript state rather than an instrument, so it opens with the
//! turn that failed and keeps its tool lines; the two panels are the exception
//! design.md §6 allows for a turn that did not complete. Both state what was
//! kept, what was billed, and what is still on disk — the questions someone
//! asks in the second after they press ctrl+c.
//!
//! Mirrors artboard `6c` of `docs/ui/Pi TUI Instruments.dc.html`. The retry
//! countdown row is drawn only when the run carries a schedule; a run the
//! agent does not retry states the failed call and stops there.

use ratatui::text::{Line, Span};

use super::sheet::{hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State};
use crate::davinci::ui::{
    blank, indent, span, span_strong, spread, truncate_run, wrap, Surface, MEASURE,
};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(run) = model.failed_run.as_ref() else {
        return vec![Line::from(vec![span(
            "the last run completed — nothing to recover",
            th.muted,
        )])];
    };

    let mut out = vec![Line::from(vec![span(
        format!("{} davinci", glyph::AGENT),
        th.primary,
    )])];

    for (state, text, detail) in &run.tools {
        out.push(indent(
            2,
            vec![
                span(format!("{} ", glyph::BRANCH), th.border),
                span_strong(format!("{} ", state.glyph()), th.state_color(*state), th),
                span(
                    text.clone(),
                    if *state == State::Failed {
                        th.error
                    } else {
                        th.muted
                    },
                ),
                span(format!(" · {detail}"), th.border),
            ],
        ));
    }
    out.push(blank());

    // The failed call, in the provider's own words, with what survived it.
    let inner = width.saturating_sub(4);
    let headline = if run.error.is_empty() {
        run.tools
            .iter()
            .rev()
            .find(|(state, _, _)| *state == State::Failed)
            .map(|(_, text, _)| text.clone())
            .unwrap_or_else(|| "the turn stopped before it finished".into())
    } else {
        run.error.clone()
    };
    let mut body: Vec<Vec<Span<'static>>> = Vec::new();
    for (index, row) in wrap(&headline, inner.saturating_sub(2))
        .into_iter()
        .enumerate()
    {
        if index == 0 {
            body.push(vec![
                span_strong(format!("{} ", glyph::FAILED), th.error, th),
                span(row, th.text),
            ]);
        } else {
            body.push(vec![span(format!("  {row}"), th.text)]);
        }
    }
    body.push(Vec::new());
    let mut ledger = vec![
        span("kept ", th.muted),
        span(run.kept.clone(), th.success),
        span(" of reply", th.muted),
    ];
    if !run.files_written.is_empty() {
        ledger.push(span("   files written ", th.muted));
        ledger.push(span(run.files_written.clone(), th.text));
    }
    if !run.billed.is_empty() {
        ledger.push(span("   billed ", th.muted));
        ledger.push(span(run.billed.clone(), th.text));
    }
    ledger.push(span("   session written ", th.muted));
    ledger.push(span_strong(glyph::DONE, th.success, th));
    body.push(ledger);
    if !run.retry.is_empty() {
        body.push(Vec::new());
        body.push(
            spread(
                inner,
                vec![
                    span(
                        format!("{} ", th.spinner(model.tick, model.animate)),
                        th.primary,
                    ),
                    span(run.retry.clone(), th.text),
                ],
                vec![
                    span("[enter]", th.primary),
                    span(" retry now", th.text),
                    span("   [m]", th.primary),
                    span(" finish on opus", th.muted),
                    span("   [esc]", th.border),
                    span(" stop retrying", th.border),
                ],
            )
            .spans,
        );
    }
    out.extend(
        Surface::new(width, th)
            .border(th.error)
            .title(vec![span("THE TURN DID NOT COMPLETE", th.error)])
            .rows(body)
            .lines(),
    );

    out.push(blank());
    out.push(Line::from(vec![
        span(format!("{} ", glyph::USER), th.primary),
        span("ctrl+c", th.border),
    ]));
    out.push(blank());

    let mut interrupted: Vec<Vec<Span<'static>>> = Vec::new();
    for (index, row) in wrap(
        "You stopped the run, not the app. The partial reply stays in the \
         transcript so the next turn can see what it was doing.",
        inner.saturating_sub(2),
    )
    .into_iter()
    .enumerate()
    {
        if index == 0 {
            interrupted.push(vec![
                span_strong(format!("{} ", glyph::ATTENTION), th.warning, th),
                span(row, th.text),
            ]);
        } else {
            interrupted.push(vec![span(format!("  {row}"), th.text)]);
        }
    }
    interrupted.push(Vec::new());
    for (state, text) in &run.aftermath {
        // The closing note carries no glyph: it is advice, not an outcome.
        if *state == State::Skipped {
            interrupted.push(vec![span(text.clone(), th.border)]);
        } else {
            interrupted.push(vec![
                span_strong(format!("{} ", state.glyph()), th.state_color(*state), th),
                span(text.clone(), th.muted),
            ]);
        }
    }
    out.extend(
        Surface::new(width, th)
            .border(th.warning)
            .title(vec![span("INTERRUPTED", th.warning)])
            .rows(interrupted)
            .lines(),
    );

    out.push(blank());
    out.extend(
        wrap(
            "Say continue and it picks up from the partial reply, or give it a \
             different instruction and the partial is treated as context, not \
             as a commitment.",
            MEASURE.min(width),
        )
        .into_iter()
        .map(|row| Line::from(vec![span(row, th.text)])),
    );
    // Loose rows are cut at the window, never past it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the transcript's own header and
/// context meter, `interrupted` in the status bar, the failed prompt echoed
/// as the first row, the composer ready with the recovery hints.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let run = model.failed_run.as_ref();
    SheetChrome {
        header_right: Vec::new(),
        status_third: run.map(|_| vec![span("interrupted", th.warning)]),
        status_right: None,
        hints: vec![
            hint(th, "enter send"),
            hint(th, "shift+enter newline"),
            hint_dim(th, "alt+enter queue for after the retry"),
        ],
        escape: Some("esc cancel"),
        composer: Composer::Prompt("continue, but do the sse path first"),
        echo: run
            .filter(|r| !r.prompt.is_empty())
            .map(|r| r.prompt.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::FailedRun;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn run() -> FailedRun {
        FailedRun {
            prompt: "rewrite the provider adapter to stream".into(),
            tools: vec![
                (
                    State::Read,
                    "read davinci-ai\\src\\openai.rs".into(),
                    "2,041 lines".into(),
                ),
                (
                    State::Done,
                    "cargo check -p davinci-ai".into(),
                    "1.84s · manus".into(),
                ),
                (
                    State::Failed,
                    "stream · 429 rate limited after 1,204 tokens".into(),
                    "0.9s".into(),
                ),
            ],
            error: "anthropic returned 429 mid-stream. Retry-After says 12s; this is \
                    attempt 2 of 4, backing off 2s, 6s, 12s."
                .into(),
            kept: "1,204 tokens".into(),
            files_written: "0".into(),
            billed: "$0.04".into(),
            retry: "retrying in 9s".into(),
            aftermath: vec![
                (
                    State::Done,
                    "transcript written to the session file · nothing to recover on restart".into(),
                ),
                (
                    State::Attention,
                    "edit to openai.rs had not started — the file on disk is untouched".into(),
                ),
                (
                    State::Skipped,
                    "a second ctrl+c within a second clears the composer; ctrl+d quits".into(),
                ),
            ],
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.failed_run = Some(run());
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_aftermath_states_what_was_kept_billed_and_still_on_disk() {
        let rows: Vec<String> = lines(&model(100)).iter().map(text).collect();
        // The prompt is the frame's echo row, not the view's.
        assert!(!rows
            .iter()
            .any(|row| row.contains("rewrite the provider adapter")));
        assert_eq!(rows[0], "◆ davinci");
        assert!(rows
            .iter()
            .any(|row| row.contains("⎿ × stream · 429 rate limited after 1,204 tokens · 0.9s")));
        assert!(rows
            .iter()
            .any(|row| row.contains("THE TURN DID NOT COMPLETE")));
        assert!(rows
            .iter()
            .any(|row| row.contains("× anthropic returned 429 mid-stream.")));
        assert!(rows.iter().any(|row| row.contains(
            "kept 1,204 tokens of reply   files written 0   billed $0.04   session written ✓"
        )));
        assert!(rows.iter().any(|row| row.contains("retrying in 9s")
            && row.contains("[enter] retry now   [m] finish on opus   [esc] stop retrying")));
        assert!(rows.iter().any(|row| row == "> ctrl+c"));
        assert!(rows.iter().any(|row| row.contains("INTERRUPTED")));
        assert!(rows
            .iter()
            .any(|row| row.contains("! You stopped the run, not the app.")));
        assert!(rows
            .iter()
            .any(|row| row.contains("the file on disk is untouched")));
        let advice = rows
            .iter()
            .find(|row| row.contains("a second ctrl+c within a second"))
            .expect("the closing note");
        assert!(!advice.contains("○"), "{advice}");
        assert!(rows.iter().any(|row| row.starts_with("Say continue")));
    }

    #[test]
    fn a_run_without_a_schedule_draws_no_countdown() {
        let mut m = model(100);
        m.failed_run.as_mut().unwrap().retry = String::new();
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(!rows.iter().any(|row| row.contains("retry now")));
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
    fn a_completed_run_says_there_is_nothing_to_recover() {
        let mut m = model(100);
        m.failed_run = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to recover")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "6c");
        let c = chrome(&m);
        assert!(c.header_right.is_empty());
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "interrupted");
        assert_eq!(c.status_right, None);
        assert_eq!(m.context, (58_000, 200_000));
        assert_eq!(c.escape, Some("esc cancel"));
        assert_eq!(
            c.composer,
            Composer::Prompt("continue, but do the sse path first")
        );
        assert_eq!(
            c.echo.as_deref(),
            Some("rewrite the provider adapter to stream")
        );
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with(
                "enter send │ shift+enter newline │ alt+enter queue for after the retry"
            ),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc cancel"), "{hint}");
    }
}
