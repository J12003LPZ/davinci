//! ChatGPT plan usage under the context bar, for `openai-codex` models: each
//! window the plan reports (the 5-hour one, the weekly one) as chatgpt.com
//! draws it, named, with when it resets and what is left, over a bar of what
//! is left. Each card is built like the context bar above it: a header row,
//! then a rule as wide as the context bar's.
//!
//! ```text
//!   ◇ 5-hour limit                               resets in 4h 27m   100% left
//!   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//!   ◇ weekly limit                                 resets in 6d 3h    84% left
//!   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┄┄┄┄┄┄┄┄┄┄┄
//! ```
//!
//! A short terminal gets one row per window instead (`◇ 5-hour limit ━━━━━━
//! 100% left · resets in 4h 27m`). When nothing can be read it says why in
//! one muted row; when the last read failed, the reason follows the numbers.
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, PlanUsageView, PlanWindow};
use crate::davinci::ui::{span, truncate_run, MEASURE};

const INDENT: &str = "  ";
/// The bar on a one-row window.
const RULE: usize = 16;
const LEFT: &str = "━";
const SPENT: &str = "┄";

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

/// The window's name the way chatgpt.com puts it: `5-hour limit`,
/// `weekly limit`; any other length by its short label (`3d limit`).
pub fn title(window: &PlanWindow) -> String {
    match window.minutes {
        300 => "5-hour limit".into(),
        10_080 => "weekly limit".into(),
        43_200 => "monthly limit".into(),
        _ => format!("{} limit", window.label),
    }
}

fn left_percent(window: &PlanWindow) -> f64 {
    (100.0 - window.used_percent).clamp(0.0, 100.0)
}

fn tone(model: &Model, left: f64) -> Color {
    let th = &model.theme;
    if left <= 10.0 {
        th.error
    } else if left <= 30.0 {
        th.warning
    } else {
        th.success
    }
}

fn resets(window: &PlanWindow) -> Option<String> {
    window.resets_in.map(|seconds| match format_reset(seconds) {
        now if now == "now" => "resets now".to_string(),
        later => format!("resets in {later}"),
    })
}

fn percent_left(model: &Model, window: &PlanWindow) -> Span<'static> {
    let left = left_percent(window);
    Span::styled(
        format!("{left:.0}% left"),
        Style::default()
            .fg(tone(model, left))
            .add_modifier(Modifier::BOLD),
    )
}

/// `cols` cells: what is left solid in its tone, what is spent dotted.
fn bar(model: &Model, window: &PlanWindow, cols: usize) -> Vec<Span<'static>> {
    let left = left_percent(window);
    let filled = ((left / 100.0) * cols as f64).round() as usize;
    // Anything left shows at least one cell; nothing left shows none.
    let filled = if left > 0.0 { filled.max(1) } else { 0 }.min(cols);
    let mut spans = Vec::with_capacity(2);
    if filled > 0 {
        spans.push(span(LEFT.repeat(filled), tone(model, left)));
    }
    if filled < cols {
        spans.push(span(SPENT.repeat(cols - filled), model.theme.cc().subtle));
    }
    spans
}

fn width_of(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn head(model: &Model, name: String) -> Vec<Span<'static>> {
    let th = &model.theme;
    vec![
        span(INDENT, th.muted),
        span("◇ ", th.cc().claude),
        Span::styled(
            name,
            Style::default().fg(th.text).add_modifier(Modifier::BOLD),
        ),
    ]
}

/// A card: the header (name left; reset and what is left right), then the
/// bar across `inner` columns. The reset goes first when there is no room.
fn card(model: &Model, window: &PlanWindow, inner: usize) -> [Line<'static>; 2] {
    let room = inner + INDENT.len();
    let left = head(model, title(window));
    let percent = percent_left(model, window);
    let with_reset = resets(window).map(|text| {
        vec![
            span(text, model.theme.muted),
            Span::raw("   "),
            percent.clone(),
        ]
    });
    let right = with_reset
        .filter(|right| width_of(&left) + 1 + width_of(right) <= room)
        .unwrap_or_else(|| vec![percent]);
    let gap = room
        .saturating_sub(width_of(&left) + width_of(&right))
        .max(1);
    let mut header = left;
    header.push(Span::raw(" ".repeat(gap)));
    header.extend(right);
    let mut rule = vec![span(INDENT, model.theme.muted)];
    rule.extend(bar(model, window, inner));
    [
        Line::from(truncate_run(header, room as u16)),
        Line::from(rule),
    ]
}

/// One row: the name padded to `name_width` so the bars line up, a short
/// bar, what is left, when it resets. The reset drops first, then the bar,
/// before the row is clipped.
fn row(model: &Model, window: &PlanWindow, name_width: usize, width: usize) -> Line<'static> {
    let name = format!("{:<name_width$}", title(window));
    let percent = percent_left(model, window);
    let reset = resets(window).map(|text| span(format!(" · {text}"), model.theme.muted));
    let build = |with_bar: bool, with_reset: bool| {
        let mut spans = head(model, name.clone());
        spans.push(Span::raw(" "));
        if with_bar {
            spans.extend(bar(model, window, RULE));
            spans.push(Span::raw(" "));
        }
        spans.push(percent.clone());
        if with_reset {
            spans.extend(reset.clone());
        }
        spans
    };
    let spans = [(true, true), (true, false), (false, false)]
        .into_iter()
        .map(|(with_bar, with_reset)| build(with_bar, with_reset))
        .find(|spans| width_of(spans) <= width)
        .unwrap_or_else(|| build(false, false));
    Line::from(truncate_run(spans, width as u16))
}

pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let Some(view) = model.plan_usage.as_ref() else {
        return Vec::new();
    };
    if height < super::context_bar::MIN_HEIGHT {
        return Vec::new();
    }
    let th = &model.theme;
    let width = usize::from(model.width.min(MEASURE + 6));
    // The context bar's rule width, so the two stacks line up.
    let inner = width.saturating_sub(INDENT.len() * 2);
    let note = view.note.as_ref();
    if view.windows.is_empty() {
        let Some(note) = note else {
            return Vec::new();
        };
        let mut spans = head(model, "plan usage".into());
        spans.push(span(format!(" · {note}"), th.muted));
        return vec![Line::from(truncate_run(spans, width as u16))];
    }
    let mut rows: Vec<Line<'static>> = if height >= super::context_bar::FULL_HEIGHT && inner >= 24 {
        view.windows
            .iter()
            .flat_map(|window| card(model, window, inner))
            .collect()
    } else {
        let name_width = view
            .windows
            .iter()
            .map(|window| title(window).chars().count())
            .max()
            .unwrap_or(0);
        view.windows
            .iter()
            .map(|window| row(model, window, name_width, width))
            .collect()
    };
    // The last read failed: the numbers above are old, and this says why.
    if let Some(note) = note {
        rows.push(Line::from(truncate_run(
            vec![
                span(INDENT, th.muted),
                span(format!("⚠ {note}"), th.warning),
            ],
            width as u16,
        )));
    }
    rows
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

    /// The account in the chatgpt.com screenshot: 5-hour untouched and
    /// resetting in 4h 27m, weekly at 84% left resetting in 6d 3h.
    fn plus() -> PlanUsageView {
        view(
            Some("plus".into()),
            vec![
                window("week", 10_080, 16.0, Some(6 * 86_400 + 3 * 3600)),
                window("5h", 300, 0.0, Some(4 * 3600 + 27 * 60)),
            ],
            None,
        )
    }

    const TALL: usize = 44;
    const SHORT: usize = 20;

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
    fn windows_are_named_like_chatgpt_names_them() {
        assert_eq!(title(&window("5h", 300, 0.0, None)), "5-hour limit");
        assert_eq!(title(&window("week", 10_080, 0.0, None)), "weekly limit");
        assert_eq!(title(&window("month", 43_200, 0.0, None)), "monthly limit");
        assert_eq!(title(&window("3d", 4_320, 0.0, None)), "3d limit");
    }

    #[test]
    fn a_tall_screen_draws_one_card_per_window_five_hour_first() {
        let m = model(100, plus());
        let rows: Vec<String> = lines(&m, TALL).iter().map(text).collect();
        assert_eq!(rows.len(), 4, "{rows:?}");
        // Header: name left; reset and what is left flush right, at the
        // context bar's width (80 columns: the measure plus its margins).
        assert!(rows[0].starts_with("  ◇ 5-hour limit "), "{rows:?}");
        assert!(
            rows[0].ends_with("resets in 4h 27m   100% left"),
            "{rows:?}"
        );
        assert!(rows[2].starts_with("  ◇ weekly limit "), "{rows:?}");
        assert!(rows[2].ends_with("resets in 6d 3h   84% left"), "{rows:?}");
        for row in [&rows[0], &rows[2]] {
            assert_eq!(row.chars().count(), 78, "{row}");
        }
        // Bars: as wide as the context bar's rule, filled with what is left.
        let inner = 76;
        for (bar, left) in [(&rows[1], 1.0), (&rows[3], 0.84)] {
            let cells = bar.trim_start();
            assert_eq!(cells.chars().count(), inner, "{bar}");
            let filled = cells.chars().filter(|c| *c == '━').count();
            assert_eq!(filled, (left * inner as f64).round() as usize, "{bar}");
            assert!(cells.chars().all(|c| c == '━' || c == '┄'), "{bar}");
        }
        assert!(!rows.concat().contains("plus"), "the plan is not a label");
    }

    #[test]
    fn a_short_screen_draws_one_aligned_row_per_window() {
        let m = model(100, plus());
        let rows: Vec<String> = lines(&m, SHORT).iter().map(text).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].starts_with("  ◇ 5-hour limit ━"), "{rows:?}");
        assert!(
            rows[0].ends_with("100% left · resets in 4h 27m"),
            "{rows:?}"
        );
        assert!(rows[1].starts_with("  ◇ weekly limit ━"), "{rows:?}");
        assert!(rows[1].ends_with("84% left · resets in 6d 3h"), "{rows:?}");
        // The bars start in one column.
        assert_eq!(rows[0].find('━'), rows[1].find('━'), "{rows:?}");
    }

    #[test]
    fn a_free_plan_month_window_reads_as_a_monthly_limit() {
        let usage = view(
            Some("free".into()),
            vec![window("month", 43_200, 53.0, Some(29 * 86_400 + 15 * 3600))],
            None,
        );
        let rows: Vec<String> = lines(&model(100, usage), TALL).iter().map(text).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].starts_with("  ◇ monthly limit"), "{rows:?}");
        assert!(
            rows[0].ends_with("resets in 29d 15h   47% left"),
            "{rows:?}"
        );
    }

    #[test]
    fn the_tone_follows_what_is_left() {
        for (used, expect) in [(95.0, "error"), (75.0, "warning"), (20.0, "success")] {
            let m = model(100, view(None, vec![window("5h", 300, used, None)], None));
            let colour = match expect {
                "error" => m.theme.error,
                "warning" => m.theme.warning,
                _ => m.theme.success,
            };
            for height in [TALL, SHORT] {
                let rows = lines(&m, height);
                let left = rows
                    .iter()
                    .flat_map(|row| row.spans.iter())
                    .find(|s| s.content.contains("% left"))
                    .unwrap();
                assert_eq!(left.style.fg, Some(colour), "{used} at {height}");
                let filled = rows
                    .iter()
                    .flat_map(|row| row.spans.iter())
                    .find(|s| s.content.contains('━'))
                    .unwrap();
                assert_eq!(filled.style.fg, Some(colour), "{used} at {height}");
            }
        }
    }

    #[test]
    fn a_spent_window_draws_no_left_cells_and_a_sliver_still_shows() {
        let spent = model(100, view(None, vec![window("5h", 300, 100.0, None)], None));
        let all: String = lines(&spent, TALL).iter().map(text).collect();
        assert!(!all.contains('━') && all.contains("0% left"), "{all}");
        let sliver = model(100, view(None, vec![window("5h", 300, 99.9, None)], None));
        let bar = text(&lines(&sliver, TALL)[1]);
        assert_eq!(bar.chars().filter(|c| *c == '━').count(), 1, "{bar}");
    }

    #[test]
    fn every_width_and_height_fits() {
        for width in [24u16, 30, 40, 56, 70, 90, 140] {
            for height in [SHORT, TALL] {
                let rows = lines(&model(width, plus()), height);
                assert!(!rows.is_empty());
                for row in &rows {
                    assert!(
                        row.width() <= usize::from(width),
                        "{width}x{height}: {}",
                        text(row)
                    );
                }
                let all: String = rows.iter().map(text).collect();
                assert!(
                    all.contains("100% left") || width < 30,
                    "{width}x{height}: {all}"
                );
            }
        }
    }

    #[test]
    fn a_narrow_card_drops_the_reset_before_what_is_left() {
        let rows: Vec<String> = lines(&model(30, plus()), TALL).iter().map(text).collect();
        assert!(rows[0].ends_with("100% left"), "{rows:?}");
        assert!(!rows[0].contains("resets"), "{rows:?}");
    }

    #[test]
    fn old_numbers_carry_the_reason_they_are_old() {
        let note = "Codex login expired · run `codex login` to see plan usage";
        let mut usage = plus();
        usage.note = Some(note.into());
        for (height, count) in [(TALL, 5), (SHORT, 3)] {
            let rows: Vec<String> = lines(&model(100, usage.clone()), height)
                .iter()
                .map(text)
                .collect();
            assert_eq!(rows.len(), count, "{rows:?}");
            assert_eq!(rows.last().unwrap(), &format!("  ⚠ {note}"), "{rows:?}");
        }
        let rows = lines(&model(44, usage), TALL);
        for row in &rows {
            assert!(row.width() <= 44, "{}", text(row));
        }
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
