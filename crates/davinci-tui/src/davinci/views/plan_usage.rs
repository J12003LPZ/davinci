//! ChatGPT plan usage under the context bar, for `openai-codex` models: the
//! 5-hour window (Plus, Pro and the like) and the weekly one (every plan),
//! each as what is left, a short rule, and when it resets.
//!
//! ```text
//!   ◇ plus   5h ━━━━━━━━━━━━──── 73% left · 2h 14m   week ━━━━━━━───────── 41% left · 3d 4h
//! ```
//!
//! Narrow screens drop the rules, then put the weekly window on its own row.
//! When nothing can be read it says why in one muted row.
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, PlanUsageView, PlanWindow};
use crate::davinci::ui::{span, MEASURE};

const INDENT: &str = "  ";
const RULE: usize = 12;

/// `2h 14m`, `3d 4h`, `45m`, `now`.
pub fn format_reset(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    if minutes == 0 {
        return "now".into();
    }
    let (days, hours, mins) = (minutes / 1440, (minutes % 1440) / 60, minutes % 60);
    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if mins > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{mins}m")
    }
}

fn window_spans(model: &Model, window: &PlanWindow, rule: bool) -> Vec<Span<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let left = (100.0 - window.used_percent).clamp(0.0, 100.0);
    let tone = if left <= 10.0 {
        th.error
    } else if left <= 30.0 {
        th.warning
    } else {
        th.success
    };
    let mut spans = vec![span(format!("{} ", window.label), th.muted)];
    if rule {
        let filled = ((left / 100.0) * RULE as f64).round() as usize;
        spans.push(span("━".repeat(filled), tone));
        spans.push(span("─".repeat(RULE - filled), cc.subtle));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        format!("{left:.0}% left"),
        Style::default().fg(tone).add_modifier(Modifier::BOLD),
    ));
    if let Some(seconds) = window.resets_in {
        spans.push(span(format!(" · {}", format_reset(seconds)), th.muted));
    }
    spans
}

fn width_of(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let Some(view) = model.plan_usage.as_ref() else {
        return Vec::new();
    };
    if height < super::context_bar::MIN_HEIGHT {
        return Vec::new();
    }
    let th = &model.theme;
    let cc = th.cc();
    let width = usize::from(model.width.min(MEASURE + 6));
    let head = |label: String| {
        vec![
            span(INDENT, th.muted),
            span("◇ ", cc.claude),
            span(label, th.text),
        ]
    };
    if view.windows.is_empty() {
        let Some(note) = &view.note else {
            return Vec::new();
        };
        let mut spans = head("plan usage".into());
        spans.push(span(format!(" · {note}"), th.muted));
        return vec![Line::from(crate::davinci::ui::truncate_run(
            spans,
            width as u16,
        ))];
    }
    let label = view.plan.clone().unwrap_or_else(|| "plan".into());
    let gap = "   ";
    let indent = " ".repeat(width_of(&head(label.clone())));
    // The last read failed: the numbers are old, and this says why.
    let note = view
        .note
        .as_ref()
        .map(|note| span(format!("{gap}⚠ {note}"), th.warning));
    let clip = |spans: Vec<Span<'static>>| {
        Line::from(crate::davinci::ui::truncate_run(spans, width as u16))
    };
    let row = |rule: bool| {
        let mut spans = head(label.clone());
        for window in &view.windows {
            spans.push(Span::raw(gap));
            spans.extend(window_spans(model, window, rule));
        }
        spans
    };
    // Everything on one row, with rules if they fit, else without.
    for rule in [true, false] {
        let mut spans = row(rule);
        spans.extend(note.clone());
        if width_of(&spans) <= width {
            return vec![Line::from(spans)];
        }
    }
    // The windows on one row, the note under them.
    let note_row = note.map(|note| clip(vec![Span::raw(indent.clone()), note]));
    for rule in [true, false] {
        let spans = row(rule);
        if width_of(&spans) <= width {
            return std::iter::once(Line::from(spans)).chain(note_row).collect();
        }
    }
    // One window per row, without rules, then the note.
    view.windows
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let mut spans = if index == 0 {
                head(label.clone())
            } else {
                vec![Span::raw(indent.clone())]
            };
            spans.push(Span::raw(gap));
            spans.extend(window_spans(model, window, false));
            clip(spans)
        })
        .chain(note_row)
        .collect()
}

/// The view for a snapshot's windows, 5-hour first.
pub fn view(plan: Option<String>, windows: Vec<PlanWindow>, note: Option<String>) -> PlanUsageView {
    let mut windows = windows;
    windows.sort_by_key(|window| window.minutes);
    PlanUsageView {
        plan,
        windows,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn window(label: &str, minutes: u64, used: f64, resets_in: Option<u64>) -> PlanWindow {
        PlanWindow {
            label: label.into(),
            minutes,
            used_percent: used,
            resets_in,
        }
    }

    fn model(width: u16, usage: PlanUsageView) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.plan_usage = Some(usage);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn resets_read_as_short_durations() {
        assert_eq!(format_reset(0), "now");
        assert_eq!(format_reset(30), "1m");
        assert_eq!(format_reset(45 * 60), "45m");
        assert_eq!(format_reset(2 * 3600 + 14 * 60), "2h 14m");
        assert_eq!(format_reset(3 * 3600), "3h");
        assert_eq!(format_reset(3 * 86_400 + 4 * 3600 + 59), "3d 4h");
        assert_eq!(format_reset(86_400), "1d");
    }

    #[test]
    fn a_plus_plan_shows_both_windows_five_hour_first() {
        let usage = view(
            Some("plus".into()),
            vec![
                window("week", 10_080, 59.0, Some(5 * 86_400)),
                window("5h", 300, 27.0, Some(2 * 3600 + 14 * 60)),
            ],
            None,
        );
        let rows: Vec<String> = lines(&model(130, usage), 40).iter().map(text).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert!(row.starts_with("  ◇ plus"), "{row}");
        let five = row.find("5h ").unwrap();
        let week = row.find("week ").unwrap();
        assert!(five < week, "{row}");
        assert!(row.contains("73% left · 2h 14m"), "{row}");
        assert!(row.contains("41% left · 5d"), "{row}");
        assert!(row.contains('━'));
    }

    #[test]
    fn a_free_plan_shows_only_the_weekly_window() {
        let usage = view(
            Some("free".into()),
            vec![window("week", 10_080, 12.0, None)],
            None,
        );
        let rows: Vec<String> = lines(&model(100, usage), 40).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("week") && rows[0].contains("88% left"));
        assert!(!rows[0].contains("5h"));
    }

    #[test]
    fn the_tone_follows_what_is_left() {
        let m = model(120, view(None, vec![window("5h", 300, 95.0, None)], None));
        let row = &lines(&m, 40)[0];
        let left = row
            .spans
            .iter()
            .find(|s| s.content.contains("% left"))
            .unwrap();
        assert_eq!(left.style.fg, Some(m.theme.error));
        let m = model(120, view(None, vec![window("5h", 300, 75.0, None)], None));
        let row = &lines(&m, 40)[0];
        let left = row
            .spans
            .iter()
            .find(|s| s.content.contains("% left"))
            .unwrap();
        assert_eq!(left.style.fg, Some(m.theme.warning));
    }

    #[test]
    fn narrow_screens_drop_rules_then_stack_and_always_fit() {
        let usage = view(
            Some("plus".into()),
            vec![
                window("5h", 300, 27.0, Some(8040)),
                window("week", 10_080, 59.0, Some(400_000)),
            ],
            None,
        );
        for width in [40u16, 56, 70, 90, 140] {
            let rows = lines(&model(width, usage.clone()), 40);
            assert!(!rows.is_empty());
            for row in &rows {
                assert!(row.width() <= usize::from(width), "{width}: {}", text(row));
            }
            let all: String = rows.iter().map(text).collect();
            assert!(all.contains("73% left") || width < 50, "{width}: {all}");
        }
    }

    #[test]
    fn old_numbers_carry_the_reason_they_are_old() {
        let note = "Codex login expired · run `codex login` to see plan usage";
        let usage = view(
            Some("plus".into()),
            vec![
                window("5h", 300, 27.0, None),
                window("week", 10_080, 59.0, None),
            ],
            Some(note.into()),
        );
        let rows: Vec<String> = lines(&model(200, usage.clone()), 40)
            .iter()
            .map(text)
            .collect();
        // Too long for one row at the content measure: the note goes under.
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(
            rows[0].contains("73% left") && rows[0].contains('━'),
            "{rows:?}"
        );
        assert!(rows[1].trim_start() == format!("⚠ {note}"), "{rows:?}");
        // A short note fits on the row.
        let short = view(
            None,
            vec![window("week", 10_080, 59.0, None)],
            Some("retrying".into()),
        );
        let rows: Vec<String> = lines(&model(200, short), 40).iter().map(text).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].ends_with("41% left   ⚠ retrying"), "{rows:?}");
        // Stacked on a narrow screen: the note gets its own row, clipped.
        let rows = lines(&model(44, usage), 40);
        let last = text(rows.last().unwrap());
        assert!(last.contains("⚠ Codex login"), "{last}");
        for row in &rows {
            assert!(row.width() <= 44, "{}", text(row));
        }
    }

    #[test]
    fn a_free_plan_month_window_reads_as_a_month() {
        let usage = view(
            Some("free".into()),
            vec![window("month", 43_200, 0.0, Some(29 * 86_400))],
            None,
        );
        let rows: Vec<String> = lines(&model(100, usage), 40).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].starts_with("  ◇ free")
                && rows[0].contains("month")
                && rows[0].contains("100% left · 29d"),
            "{rows:?}"
        );
    }

    #[test]
    fn nothing_read_yet_says_why_or_stays_quiet() {
        let m = model(
            100,
            view(
                None,
                Vec::new(),
                Some("Codex login expired · run `codex login` to see plan usage".into()),
            ),
        );
        let rows: Vec<String> = lines(&m, 40).iter().map(text).collect();
        assert_eq!(
            rows,
            ["  ◇ plan usage · Codex login expired · run `codex login` to see plan usage"]
        );
        let m = model(100, view(None, Vec::new(), None));
        assert!(lines(&m, 40).is_empty());
        let mut m = model(100, view(None, vec![window("5h", 300, 1.0, None)], None));
        assert!(lines(&m, super::super::context_bar::MIN_HEIGHT - 1).is_empty());
        m.plan_usage = None;
        assert!(lines(&m, 40).is_empty());
    }
}
