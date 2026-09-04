//! `4c` — `/compact`. What compaction would keep, what it would fold away, and
//! what it costs — stated before it happens, like the governor proposal in
//! `2c` (design.md §6). It never acts silently.
//!
//! The cost the screen exists to make visible is the one nobody expects:
//! folding the context re-primes the prompt cache, so the next turn pays a
//! full cache write before it reads a single new token.
//!
//! Mirrors artboard `4c` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use super::sheet::{facts, Composer, SheetChrome};
use crate::davinci::model::{Compaction, Model};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{blank, footnote, meter, span, span_strong, truncate_run, wrap, Surface};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

    let Some(plan) = &model.compaction else {
        return vec![Line::from(vec![span(
            "nothing to compact — the session is empty",
            th.muted,
        )])];
    };

    let mut out = vec![
        meter_row(
            "now",
            &plan.before_tokens,
            plan.before_fraction,
            &plan.before_note,
            th.warning,
            th,
        ),
        meter_row(
            "after",
            &plan.after_tokens,
            plan.after_fraction,
            &plan.after_note,
            th.success,
            th,
        ),
        blank(),
    ];

    let kept: Vec<Vec<Span<'static>>> = plan
        .kept
        .iter()
        .map(|text| {
            vec![
                span_strong(format!("{} ", glyph::DONE), th.success, th),
                span(text.clone(), th.muted),
            ]
        })
        .collect();
    let mut folded: Vec<Vec<Span<'static>>> = plan
        .folded
        .iter()
        .map(|text| vec![span("− ", th.error), span(text.clone(), th.muted)])
        .collect();
    if !plan.note_cost.is_empty() {
        folded.push(vec![span(
            format!("the note itself costs about {}", plan.note_cost),
            th.border,
        )]);
    }
    // Side by side where the window allows it, as the artboard sets them;
    // stacked below 100 columns (design.md §7).
    if width >= 100 {
        let half = (width - 2) / 2;
        let left = Surface::new(half, th)
            .title(vec![span("KEPT VERBATIM", th.success)])
            .rows(kept)
            .lines();
        let right = Surface::new(width - half - 2, th)
            .title(vec![span("FOLDED INTO ONE NOTE", th.error)])
            .rows(folded)
            .lines();
        let rows = left.len().max(right.len());
        let blank_left = Line::from(vec![span(" ".repeat(half as usize), th.border)]);
        for index in 0..rows {
            let mut spans = left.get(index).unwrap_or(&blank_left).spans.clone();
            spans.push(span("  ", th.border));
            if let Some(row) = right.get(index) {
                spans.extend(row.spans.iter().cloned());
            }
            out.push(Line::from(spans));
        }
    } else {
        out.extend(
            Surface::new(width, th)
                .title(vec![span("KEPT VERBATIM", th.success)])
                .rows(kept)
                .lines(),
        );
        out.push(blank());
        out.extend(
            Surface::new(width, th)
                .title(vec![span("FOLDED INTO ONE NOTE", th.error)])
                .rows(folded)
                .lines(),
        );
    }
    out.push(blank());

    out.extend(cost_box(plan, width, th));
    out.push(blank());
    let history = if plan.history.is_empty() {
        Vec::new()
    } else {
        vec![span(plan.history.clone(), th.muted)]
    };
    out.extend(footnote(
        width,
        history,
        vec![span("/tree still shows every folded turn", th.border)],
        th,
    ));
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the policy in the header, one proposal
/// in the status bar, the keys inside the cost panel rather than a hint row,
/// the composer ready for the next thing.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let plan = model.compaction.as_ref();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                plan.map(|plan| {
                    vec![
                        span("auto-compact ", th.muted),
                        span(if plan.auto { "on" } else { "off" }, th.text),
                    ]
                })
                .unwrap_or_default(),
                plan.filter(|plan| !plan.threshold.is_empty())
                    .map(|plan| vec![span(format!("threshold {}", plan.threshold), th.muted)])
                    .unwrap_or_default(),
                if model.model_name.is_empty() {
                    Vec::new()
                } else {
                    vec![span(model.model_name.clone(), th.muted)]
                },
            ],
        ),
        status_third: plan.map(|_| vec![span("1 proposal", th.muted)]),
        status_right: None,
        hints: Vec::new(),
        escape: None,
        composer: Composer::Prompt("ask davinci…"),
        echo: Some(
            plan.filter(|plan| !plan.command.is_empty())
                .map(|plan| plan.command.clone())
                .unwrap_or_else(|| "/compact".into()),
        ),
    }
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
        span("   reversible ", th.muted),
        span_strong(glyph::DONE, th.success, th),
        span(" the jsonl keeps every turn", th.border),
    ]);
    body.push(Vec::new());
    body.push(vec![
        span("[enter]", th.primary),
        span(" compact now", th.text),
        span("   [e]", th.primary),
        span(" evict tool output only", th.muted),
        span("   [t]", th.primary),
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
        model.model_name = "sonnet".into();
        model.compaction = Some(fixtures::compaction());
        model.toggle_screen(Screen::Compact);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_two_meters_carry_their_caps_and_the_panels_sit_side_by_side() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows[0].starts_with("now"), "{}", rows[0]);
        assert!(rows[0].contains("92% of 200k"), "{}", rows[0]);
        assert!(rows[1].starts_with("after"), "{}", rows[1]);
        let panels = rows
            .iter()
            .find(|row| row.contains("KEPT VERBATIM"))
            .unwrap();
        assert!(
            panels.contains("FOLDED INTO ONE NOTE"),
            "both panels on one rule at 100 columns: {panels}"
        );
        assert!(rows.iter().any(|row| row.contains("− turns 1–18")));
        assert!(rows
            .iter()
            .any(|row| row.contains("the note itself costs about 1.4k")));
    }

    #[test]
    fn narrow_windows_stack_the_panels() {
        let m = model(80);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let kept = rows
            .iter()
            .position(|row| row.contains("KEPT VERBATIM"))
            .unwrap();
        let folded = rows
            .iter()
            .position(|row| row.contains("FOLDED INTO ONE NOTE"))
            .unwrap();
        assert!(folded > kept + 1);
    }

    #[test]
    fn the_cost_box_states_the_cache_write_and_its_keys() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("WHAT THIS COSTS YOU")));
        assert!(rows
            .iter()
            .any(|row| row.contains("re-primes the prompt cache")));
        assert!(rows
            .iter()
            .any(|row| row.contains("[enter] compact now") && row.contains("[esc] leave it")));
        assert!(rows
            .iter()
            .any(|row| row.contains("compacted 2× this session")
                && row.contains("/tree still shows every folded turn")));
    }

    #[test]
    fn an_empty_session_has_nothing_to_compact() {
        let mut m = model(100);
        m.compaction = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("nothing to compact")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "4c");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "auto-compact on │ threshold 92% │ sonnet");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "1 proposal");
        assert_eq!(c.escape, None, "the keys live in the cost panel");
        assert_eq!(c.composer, Composer::Prompt("ask davinci…"));
        assert_eq!(
            c.echo.as_deref(),
            Some("/compact keep the store.rs decisions verbatim")
        );
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
