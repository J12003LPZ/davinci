//! `5c` — `governor-status`. What the governor did to your tool output, which
//! is a different question from `2c`'s "where is the budget going".
//!
//! Counters carry their denominator, and the screen shows one compressed
//! result in full so the elision marker and the retrieval id are visible
//! rather than described: nothing is deleted, the tail is on disk, and the
//! model can ask for any range of it back.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/governor.ex`.

use ratatui::style::Color;
use ratatui::text::Line;

use crate::davinci::model::{Model, Tone};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, span, span_strong, truncate_run, wrap, Surface, MEASURE,
};

const ID: usize = 12;
const TOOL: usize = 12;

fn tone_color(tone: Tone, theme: &Theme) -> Color {
    match tone {
        Tone::Primary => theme.primary,
        Tone::Secondary => theme.secondary,
        Tone::Warning => theme.warning,
        Tone::Success => theme.success,
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(status) = model.governor.as_ref() else {
        return vec![Line::from(vec![span(
            "the governor has nothing to report — no tool output was compressed",
            th.muted,
        )])];
    };

    let counters: Vec<Line<'static>> = status
        .counters
        .iter()
        .map(|counter| {
            Line::from(vec![
                span(
                    format!("{:>6}", counter.number),
                    tone_color(counter.tone, th),
                ),
                span(format!(" {:<14}", counter.of), th.muted),
                span(format!("{:<22}", counter.verb), th.text),
                span(counter.note.clone(), th.border),
            ])
        })
        .collect();

    let sample = Surface::new(width, th)
        .title(vec![span(
            "WHAT A COMPRESSED RESULT LOOKS LIKE",
            th.warning,
        )])
        .rows(vec![
            vec![
                span(format!("{} ", glyph::BRANCH), th.border),
                span_strong(format!("{} ", State::Done.glyph()), th.success, th),
                span("cargo test --workspace", th.muted),
                span("  41.2s · manus", th.border),
            ],
            vec![span("  running 212 tests", th.border)],
            vec![
                span("  test session::store::roundtrip ", th.border),
                span("... ok", th.success),
            ],
            vec![
                span("  … 1,184 lines held on disk · ", th.border),
                span("out-9f21c4", th.warning),
                span(" · 84 KB", th.border),
            ],
            vec![
                span("  test result: ", th.border),
                span("ok", th.success),
                span(". 212 passed; 0 failed", th.border),
            ],
            Vec::new(),
            vec![span(
                "  retrieve_output out-9f21c4 --lines 600-640",
                th.text,
            )],
            vec![
                span(format!("{} ", glyph::BRANCH), th.border),
                span_strong(format!("{} ", State::Attention.glyph()), th.warning, th),
                span("the model asked for the middle", th.muted),
            ],
        ])
        .lines();

    let header = Line::from(vec![
        span(
            format!("{:<width$}", "HELD ON DISK", width = ID + 1),
            th.border,
        ),
        span(format!("{:<width$}", "TOOL", width = TOOL + 1), th.border),
        span(format!("{:<30}", "CALL"), th.border),
        span("SIZE", th.border),
    ]);

    let stored: Vec<Line<'static>> = status
        .stored
        .iter()
        .map(|entry| {
            let color = if entry.stale { th.border } else { th.muted };
            Line::from(vec![
                span(
                    format!("{:<width$}", entry.id, width = ID + 1),
                    if entry.stale { th.border } else { th.warning },
                ),
                span(format!("{:<width$}", entry.tool, width = TOOL + 1), color),
                span(format!("{:<31}", clip_ellipsis(&entry.call, 30)), color),
                span(entry.size.clone(), th.border),
            ])
        })
        .collect();

    let mut footer: Vec<Line<'static>> = wrap(
        "compresses above 8 KB or 300 lines, keeping 40 head, 40 tail and 20 lines it judges important. Nothing is deleted.",
        MEASURE,
    )
    .into_iter()
    .map(|row| Line::from(vec![span(row, th.muted)]))
    .collect();
    footer.push(Line::from(vec![span(status.store_dir.clone(), th.border)]));
    footer.push(Line::from(vec![
        span("enter open an output", th.border),
        span(" · ", th.border),
        span("d dedupe on/off", th.border),
        span(" · ", th.border),
        span("l anti-loop on/off", th.border),
    ]));
    footer.push(Line::from(vec![
        span("r reset counters", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));

    let mut out = counters;
    out.push(blank());
    out.extend(sample);
    out.push(blank());
    out.push(header);
    out.extend(stored);
    out.push(blank());
    out.extend(footer);
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
    use crate::davinci::model::{GovernorCounter, GovernorSheet, GovernorStored};
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.governor = Some(GovernorSheet {
            counters: vec![
                GovernorCounter {
                    number: "31".into(),
                    of: "of 96 results".into(),
                    verb: "compressed".into(),
                    note: "head 40 · tail 40 · rest on disk".into(),
                    tone: Tone::Primary,
                },
                GovernorCounter {
                    number: "96.2k".into(),
                    of: "of 200k".into(),
                    verb: "tokens never sent".into(),
                    note: "about $0.29 at sonnet input".into(),
                    tone: Tone::Success,
                },
            ],
            stored: vec![
                GovernorStored {
                    id: "out-9f21c4".into(),
                    tool: "bash".into(),
                    call: "cargo test --workspace".into(),
                    size: "1,184 ln · 84 KB".into(),
                    stale: false,
                },
                GovernorStored {
                    id: "out-77b3e5".into(),
                    tool: "powershell".into(),
                    call: "git log --stat -n 40".into(),
                    size: "498 ln · 22 KB".into(),
                    stale: true,
                },
            ],
            store_dir: "%USERPROFILE%\\.davinci\\outputs\\01JB2K\\ · dropped when the session ends"
                .into(),
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
    fn every_counter_carries_its_denominator() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("31") && row.contains("of 96 results")));
        assert!(rows
            .iter()
            .any(|row| row.contains("96.2k") && row.contains("of 200k")));
    }

    #[test]
    fn the_compressed_sample_shows_the_elision_and_the_retrieval_id() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("… 1,184 lines held on disk")));
        assert!(rows
            .iter()
            .any(|row| row.contains("retrieve_output out-9f21c4")));
    }

    #[test]
    fn a_stale_stored_output_dims_while_a_live_one_keeps_its_id_in_warning() {
        let m = model(100);
        let rows = lines(&m);
        let live = rows
            .iter()
            .find(|row| text(row).starts_with("out-9f21c4"))
            .expect("live row");
        assert_eq!(live.spans[0].style.fg, Some(m.theme.warning));
        let stale = rows
            .iter()
            .find(|row| text(row).starts_with("out-77b3e5"))
            .expect("stale row");
        assert_eq!(stale.spans[0].style.fg, Some(m.theme.border));
    }

    #[test]
    fn with_nothing_governed_the_screen_says_so() {
        let mut m = model(100);
        m.governor = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("nothing to report"));
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
