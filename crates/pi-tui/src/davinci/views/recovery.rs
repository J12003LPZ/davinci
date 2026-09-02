//! `6c` — what ctrl+c actually did, and what the provider did before it.
//!
//! This is a transcript state rather than an instrument, so it opens with the
//! turn that failed and keeps its tool lines; the two panels are the exception
//! design.md §6 allows for a turn that did not complete. Both state what was
//! kept, what was billed, and what is still on disk — the questions someone
//! asks in the second after they press ctrl+c.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/recovery.ex`. The mockup's
//! retry countdown (`retrying in 9s · [enter] retry now · [m] finish on
//! opus`) belongs to a retry loop the model does not carry, so the failure
//! panel here states the failed call itself instead of a schedule it does not
//! have.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::theme::{glyph, State};
use crate::davinci::ui::{blank, indent, span, span_strong, wrap, Surface, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(run) = model.failed_run.as_ref() else {
        return vec![Line::from(vec![span(
            "the last run completed — nothing to recover",
            th.muted,
        )])];
    };

    let mut out = vec![
        Line::from(vec![
            span(format!("{} ", glyph::USER), th.primary),
            span(run.prompt.clone(), th.muted),
        ]),
        blank(),
        Line::from(vec![span(format!("{} davinci", glyph::AGENT), th.primary)]),
    ];

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
                span(format!("  {detail}"), th.border),
            ],
        ));
    }
    out.push(blank());

    // The failed call, in the provider's own words, with what survived it.
    let failed = run
        .tools
        .iter()
        .rev()
        .find(|(state, _, _)| *state == State::Failed);
    let headline = failed
        .map(|(_, text, _)| format!("{} {}", glyph::FAILED, text))
        .unwrap_or_else(|| format!("{} the turn stopped before it finished", glyph::FAILED));
    let mut body: Vec<Vec<Span<'static>>> = wrap(&headline, width.saturating_sub(6))
        .into_iter()
        .map(|row| vec![span(row, th.text)])
        .collect();
    body.push(Vec::new());
    body.push(vec![
        span("kept ", th.muted),
        span(run.kept.clone(), th.success),
        span(" of reply   billed ", th.muted),
        span(run.billed.clone(), th.text),
    ]);
    body.push(vec![
        span("session written ", th.muted),
        span_strong(glyph::DONE, th.success, th),
    ]);
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

    let mut interrupted: Vec<Vec<Span<'static>>> = wrap(
        "! You stopped the run, not the app. The partial reply stays in the \
         transcript so the next turn can see what it was doing.",
        width.saturating_sub(6),
    )
    .into_iter()
    .map(|row| vec![span(row, th.text)])
    .collect();
    interrupted.push(Vec::new());
    for (state, text) in &run.aftermath {
        interrupted.push(vec![
            span_strong(format!("{} ", state.glyph()), th.state_color(*state), th),
            span(text.clone(), th.muted),
        ]);
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
            "Say continue and it picks up from the partial reply; give it a \
             different instruction and the partial is context, not a commitment.",
            width.saturating_sub(2).min(MEASURE),
        )
        .into_iter()
        .map(|row| Line::from(vec![span(row, th.text)])),
    );
    // Loose rows are cut at the window, never past it.
    out.into_iter()
        .map(|line| Line::from(crate::davinci::ui::truncate_run(line.spans, model.width)))
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
            kept: "1,204 tokens".into(),
            billed: "$0.04".into(),
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
        assert!(rows
            .iter()
            .any(|row| row.contains("rewrite the provider adapter")));
        assert!(rows.iter().any(|row| row.contains("429 rate limited")));
        assert!(rows
            .iter()
            .any(|row| row.contains("THE TURN DID NOT COMPLETE")));
        assert!(rows.iter().any(|row| row.contains("kept 1,204 tokens")));
        assert!(rows.iter().any(|row| row.contains("billed $0.04")));
        assert!(rows.iter().any(|row| row.contains("INTERRUPTED")));
        assert!(rows
            .iter()
            .any(|row| row.contains("the file on disk is untouched")));
        assert!(rows.iter().any(|row| row.contains("Say continue")));
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
}
