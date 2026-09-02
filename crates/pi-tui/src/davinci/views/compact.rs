//! `4c` — `/compact`. What compaction would keep, what it would fold away, and
//! what it costs — stated before it happens, like the governor proposal in
//! `2c` (design.md §6). It never acts silently.
//!
//! The cost the screen exists to make visible is the one nobody expects:
//! folding the context re-primes the prompt cache, so the next turn pays a
//! full cache write before it reads a single new token.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/compact.ex`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Compaction, Model};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{blank, meter, span, span_strong, wrap, Surface, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    let Some(plan) = &model.compaction else {
        return vec![
            Line::from(vec![span(
                "nothing to compact — the session is empty",
                th.muted,
            )]),
            Line::from(vec![span("esc close", th.border)]),
        ];
    };

    let echo = Line::from(vec![
        span(format!("{} ", glyph::USER), th.primary),
        span("/compact", th.muted),
    ]);

    let mut out = vec![echo, blank()];
    out.push(meter_row(
        "now",
        &plan.before_tokens,
        plan.before_fraction,
        &plan.before_note,
        th.warning,
        th,
    ));
    out.push(meter_row(
        "after",
        &plan.after_tokens,
        plan.after_fraction,
        &plan.after_note,
        th.success,
        th,
    ));
    out.push(blank());

    out.extend(
        Surface::new(width, th)
            .title(vec![span("KEPT VERBATIM", th.success)])
            .rows(
                plan.kept
                    .iter()
                    .map(|text| {
                        vec![
                            span_strong(format!("{} ", glyph::DONE), th.success, th),
                            span(text.clone(), th.muted),
                        ]
                    })
                    .collect(),
            )
            .lines(),
    );
    out.push(blank());

    out.extend(
        Surface::new(width, th)
            .title(vec![span("FOLDED INTO ONE NOTE", th.error)])
            .right(vec![span("retrievable by id", th.border)])
            .rows(
                plan.folded
                    .iter()
                    .map(|text| {
                        vec![
                            span_strong(format!("{} ", glyph::FAILED), th.error, th),
                            span(text.clone(), th.muted),
                        ]
                    })
                    .collect(),
            )
            .lines(),
    );
    out.push(blank());

    out.extend(cost_box(plan, width, th));
    out.push(blank());
    out.push(Line::from(vec![span(
        "/tree still shows every folded turn",
        th.border,
    )]));
    out
}

fn cost_box(plan: &Compaction, width: u16, th: &Theme) -> Vec<Line<'static>> {
    let mut body: Vec<Vec<Span<'static>>> = wrap(
        &format!(
            "! compaction re-primes the prompt cache. The next turn pays a full \
             {} cache write before it reads a single new token.",
            plan.after_tokens
        ),
        width.saturating_sub(6),
    )
    .into_iter()
    .map(|row| vec![span(row, th.text)])
    .collect();
    body.push(Vec::new());
    body.push(vec![
        span("recovers ", th.muted),
        span(plan.recovers.clone(), th.success),
        span("   summarising call ", th.muted),
        span(plan.call_cost.clone(), th.text),
        span("   cache write ", th.muted),
        span(plan.cache_cost.clone(), th.text),
    ]);
    body.push(vec![
        span("reversible ", th.muted),
        span_strong(glyph::DONE, th.success, th),
        span("  the jsonl keeps every turn", th.border),
    ]);
    body.push(Vec::new());
    body.push(vec![
        span("[enter]", th.primary),
        span(" compact now", th.text),
        span("   [e]", th.primary),
        span(" evict tool output only", th.muted),
    ]);
    body.push(vec![
        span("[t]", th.primary),
        span(" raise the threshold", th.muted),
        span("   [esc]", th.border),
        span(" leave it", th.border),
    ]);

    Surface::new(width, th)
        .border(th.warning)
        .title(vec![span("WHAT THIS COSTS YOU", th.warning)])
        .rows(body)
        .lines()
}

fn meter_row(
    label: &str,
    tokens: &str,
    fraction: f64,
    note: &str,
    color: Color,
    th: &Theme,
) -> Line<'static> {
    let note_color = if color == th.warning {
        th.warning
    } else {
        th.border
    };
    let mut spans = vec![
        span(format!("{label:<7}"), th.muted),
        span(format!("{tokens:>7}  "), th.text),
    ];
    spans.extend(meter(fraction, 24, th, Some(color)));
    spans.push(span(format!("  {note}"), note_color));
    Line::from(spans)
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

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.compaction = Some(Compaction {
            before_tokens: "184.2k".into(),
            before_fraction: 0.92,
            before_note: "! 92% of 200k".into(),
            after_tokens: "61.8k".into(),
            after_fraction: 0.31,
            after_note: "31% of 200k".into(),
            kept: vec![
                "the last 6 turns, whole".into(),
                "every Δ and its hunks · 7 files".into(),
            ],
            folded: vec![
                "turns 1–18 · 96.4k".into(),
                "31 tool results · kept as ids, retrievable".into(),
            ],
            recovers: "122.4k".into(),
            call_cost: "$0.19".into(),
            cache_cost: "$0.23".into(),
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
    fn the_sheet_states_both_sides_and_the_cost() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        for expected in [
            "now",
            "184.2k",
            "after",
            "61.8k",
            "KEPT VERBATIM",
            "the last 6 turns, whole",
            "FOLDED INTO ONE NOTE",
            "retrievable by id",
            "WHAT THIS COSTS YOU",
            "recovers",
            "122.4k",
            "$0.19",
            "[enter]",
            "/tree still shows every folded turn",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(expected)),
                "{expected} is missing"
            );
        }
    }

    #[test]
    fn the_breaching_meter_note_is_drawn_in_warning() {
        let m = model(100);
        let rows = lines(&m);
        let now = rows
            .iter()
            .find(|row| text(row).contains("92% of 200k"))
            .expect("the before meter");
        assert_eq!(now.spans.last().unwrap().style.fg, Some(m.theme.warning));
        let after = rows
            .iter()
            .find(|row| text(row).contains("31% of 200k"))
            .expect("the after meter");
        assert_eq!(after.spans.last().unwrap().style.fg, Some(m.theme.border));
    }

    #[test]
    fn with_nothing_to_compact_the_sheet_says_so() {
        let mut m = model(100);
        m.compaction = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to compact")));
    }

    #[test]
    fn no_row_overflows_and_the_boxes_are_row_exact() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            let sheet = width.min(MEASURE + 14);
            let rows = lines(&m);
            let kept = rows
                .iter()
                .find(|row| text(row).contains("KEPT VERBATIM"))
                .expect("the kept box");
            assert_eq!(run_width(&kept.spans), sheet, "at {width}");
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
