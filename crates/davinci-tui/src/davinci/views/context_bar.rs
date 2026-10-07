//! The context bar: how full the context window is, pinned above the input
//! in DaVinci's ink. One heavy rule split by category in the `/context`
//! colours, a tick where auto-compact fires (the configured threshold, as a
//! percentage or a token count), and the remaining window after it drawn as
//! the reserve it is. `compact` is the header and the rule; `full` adds a
//! legend of the categories. `/config` → Context bar.
//!
//! ```text
//!   ◆ context                        90k of 1M · compacts at 987k   9%
//!   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┃┄┄
//!   ■ system prompt 4.2k 0%   ■ tools 17k 2%   ■ messages 61k 6%   free 897k
//! ```
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::context_usage::{format_tokens, kind_color};
use crate::davinci::model::{ContextBarMode, ContextUsageView, Model};
use crate::davinci::ui::{span, MEASURE};

const INDENT: &str = "  ";
const USED: &str = "━";
const FREE: &str = "━";
const RESERVE: &str = "┄";
const TICK: &str = "┃";
/// Below this many rows the bar gives the transcript its room back.
pub const MIN_HEIGHT: usize = 18;
/// Below this many rows `full` draws as `compact`.
pub const FULL_HEIGHT: usize = 28;
/// From this share of the threshold the bar warns; past it, compaction is due.
const WARN_AT: f64 = 0.8;

/// The token count above which auto-compact fires, or `None` when it is off.
pub fn compacts_at(view: &ContextUsageView) -> Option<u64> {
    (view.buffer > 0).then(|| view.window.saturating_sub(view.buffer))
}

/// How close the context is to compaction: `used / threshold` (or of the
/// window when auto-compact is off).
pub fn pressure(view: &ContextUsageView) -> f64 {
    let limit = compacts_at(view).unwrap_or(view.window);
    if limit == 0 {
        0.0
    } else {
        view.used() as f64 / limit as f64
    }
}

pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let Some(view) = model.context_meter.as_ref() else {
        return Vec::new();
    };
    if model.context_bar == ContextBarMode::Off || height < MIN_HEIGHT || view.window == 0 {
        return Vec::new();
    }
    let width = usize::from(model.width.min(MEASURE + 6)).saturating_sub(INDENT.len() * 2);
    if width < 24 {
        return Vec::new();
    }
    let mut rows = vec![header(model, view, width), rule(model, view, width)];
    if model.context_bar == ContextBarMode::Full && height >= FULL_HEIGHT {
        rows.extend(legend(model, view, width));
    }
    rows
}

fn header(model: &Model, view: &ContextUsageView, width: usize) -> Line<'static> {
    let th = &model.theme;
    let cc = th.cc();
    let used = view.used();
    let level = pressure(view);
    let tone = if level >= 1.0 {
        th.error
    } else if level >= WARN_AT {
        th.warning
    } else {
        th.muted
    };
    let left = vec![
        span(INDENT, th.muted),
        span("◆ ", cc.claude),
        Span::styled(
            "context",
            Style::default().fg(th.text).add_modifier(Modifier::BOLD),
        ),
    ];
    let bold = |text: String, color| {
        Span::styled(
            text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )
    };
    let mut compaction = Vec::new();
    match compacts_at(view) {
        Some(limit) if used >= limit => {
            compaction.push(span(" · compacting next turn", th.error));
        }
        Some(limit) => {
            compaction.push(span(" · compacts at ", th.muted));
            compaction.push(span(
                format_tokens(limit),
                if level >= WARN_AT {
                    th.warning
                } else {
                    th.muted
                },
            ));
        }
        None => compaction.push(span(" · auto-compact off", th.muted)),
    }
    let share = (used as f64 * 100.0 / view.window as f64).round() as u64;
    let percent = bold(format!("  {share}%"), tone);
    let total = span(format!(" of {}", format_tokens(view.window)), th.muted);
    // Narrow terminals lose detail from the middle; the share always stays.
    let candidates = [
        [
            vec![bold(format_tokens(used), th.text), total.clone()],
            compaction.clone(),
        ]
        .concat(),
        [vec![bold(format_tokens(used), th.text)], compaction].concat(),
        vec![bold(format_tokens(used), th.text), total],
        Vec::new(),
    ];
    let left_width: usize = left.iter().map(Span::width).sum();
    let room = width + INDENT.len();
    let mut right = candidates
        .into_iter()
        .find(|spans| {
            left_width + 1 + spans.iter().map(Span::width).sum::<usize>() + percent.width() <= room
        })
        .unwrap_or_default();
    right.push(percent);
    let used_cols = left.iter().chain(&right).map(|s| s.width()).sum::<usize>();
    let gap = (width + INDENT.len()).saturating_sub(used_cols).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}

fn rule(model: &Model, view: &ContextUsageView, width: usize) -> Line<'static> {
    let th = &model.theme;
    let cc = th.cc();
    let window = view.window as f64;
    let scale = |tokens: u64| ((tokens as f64 / window) * width as f64).round() as usize;
    let tick = compacts_at(view).map(|limit| scale(limit).min(width - 1));
    let mut colors: Vec<Color> = Vec::with_capacity(width);
    for category in view
        .categories
        .iter()
        .filter(|category| category.tokens > 0)
    {
        let n = scale(category.tokens).max(1);
        colors.extend(std::iter::repeat_n(kind_color(&cc, category.kind), n));
    }
    // A context past the window still draws inside it, with the tick shown.
    colors.truncate(width);
    let mut spans = vec![span(INDENT, th.muted)];
    let tone = if pressure(view) >= WARN_AT {
        th.warning
    } else {
        cc.inactive
    };
    for column in 0..width {
        if Some(column) == tick {
            spans.push(span(TICK, tone));
        } else if let Some(color) = colors.get(column) {
            spans.push(span(USED, *color));
        } else if tick.is_some_and(|tick| column > tick) {
            spans.push(span(RESERVE, cc.subtle));
        } else {
            spans.push(span(FREE, cc.subtle));
        }
    }
    Line::from(merge(spans))
}

/// Adjacent cells of one style as one span: the line stays short to diff.
fn merge(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    for item in spans {
        match out.last_mut() {
            Some(last) if last.style == item.style => {
                last.content = format!("{}{}", last.content, item.content).into();
            }
            _ => out.push(item),
        }
    }
    out
}

fn legend(model: &Model, view: &ContextUsageView, width: usize) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let share = |tokens: u64| (tokens as f64 * 100.0 / view.window as f64).round() as u64;
    let mut items: Vec<Vec<Span<'static>>> = view
        .categories
        .iter()
        .filter(|category| category.tokens > 0)
        .map(|category| {
            vec![
                span("■ ", kind_color(&cc, category.kind)),
                span(category.label.to_lowercase(), th.muted),
                Span::styled(
                    format!(" {}", format_tokens(category.tokens)),
                    Style::default().fg(th.text).add_modifier(Modifier::BOLD),
                ),
                span(
                    match share(category.tokens) {
                        0 => " <1%".to_string(),
                        whole => format!(" {whole}%"),
                    },
                    th.muted,
                ),
            ]
        })
        .collect();
    items.push(vec![
        span("■ ", cc.subtle),
        span("free ", th.muted),
        Span::styled(
            format_tokens(view.free),
            Style::default().fg(th.text).add_modifier(Modifier::BOLD),
        ),
    ]);
    if view.buffer > 0 {
        items.push(vec![
            span("┄ ", cc.subtle),
            span("reserve ", th.muted),
            Span::styled(
                format_tokens(view.buffer),
                Style::default().fg(th.text).add_modifier(Modifier::BOLD),
            ),
        ]);
    }
    let mut rows = Vec::new();
    let mut line: Vec<Span<'static>> = vec![span(INDENT, th.muted)];
    let mut used = 0;
    for item in items {
        let item_width: usize = item.iter().map(Span::width).sum();
        if used > 0 && used + 3 + item_width > width {
            rows.push(Line::from(std::mem::replace(
                &mut line,
                vec![span(INDENT, th.muted)],
            )));
            used = 0;
        }
        if used > 0 {
            line.push(Span::raw("   "));
            used += 3;
        }
        used += item_width;
        line.extend(item);
    }
    rows.push(Line::from(line));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{ContextCategory, ContextKind};
    use crate::davinci::theme::{ColorDepth, Theme};

    fn view(window: u64, messages: u64, buffer: u64) -> ContextUsageView {
        let categories = vec![
            ContextCategory {
                kind: ContextKind::SystemPrompt,
                label: "System prompt".into(),
                tokens: 4_200,
            },
            ContextCategory {
                kind: ContextKind::SystemTools,
                label: "System tools".into(),
                tokens: 17_000,
            },
            ContextCategory {
                kind: ContextKind::Messages,
                label: "Messages".into(),
                tokens: messages,
            },
        ];
        let used: u64 = categories.iter().map(|c| c.tokens).sum();
        ContextUsageView {
            model: "gpt-6-luna".into(),
            window,
            free: window.saturating_sub(used + buffer),
            categories,
            buffer,
            sections: Vec::new(),
        }
    }

    fn model(mode: ContextBarMode, view: ContextUsageView) -> Model {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        model.context_bar = mode;
        model.context_meter = Some(view);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn compact_is_a_header_and_a_rule_with_the_compaction_tick() {
        // 1M window, auto-compact at 80% → reserve 200k.
        let m = model(ContextBarMode::Compact, view(1_000_000, 68_800, 200_000));
        let rows = lines(&m, 40);
        assert_eq!(rows.len(), 2);
        let head = text(&rows[0]);
        assert!(head.starts_with("  ◆ context"), "{head}");
        assert!(head.ends_with("90k of 1M · compacts at 800k  9%"), "{head}");
        let rule = text(&rows[1]);
        let cells: Vec<char> = rule.trim_start().chars().collect();
        let tick = cells.iter().position(|&c| c == '┃').expect("tick");
        // 800k of 1M sits at 80% of the rule.
        let expected = (cells.len() as f64 * 0.8).round() as usize;
        assert!(
            tick.abs_diff(expected) <= 1,
            "tick {tick} of {}",
            cells.len()
        );
        assert!(cells[tick + 1..].iter().all(|&c| c == '┄'));
        assert!(cells[..tick].iter().all(|&c| c == '━'));
    }

    #[test]
    fn full_adds_a_legend_and_falls_back_to_compact_on_a_short_screen() {
        let m = model(ContextBarMode::Full, view(1_000_000, 68_800, 200_000));
        let rows: Vec<String> = lines(&m, 40).iter().map(text).collect();
        assert!(rows.len() >= 3, "{rows:?}");
        let legend = rows[2..].join("\n");
        for part in [
            "system prompt 4.2k",
            "system tools 17k 2%",
            "messages 68.8k 7%",
            "free",
            "reserve 200k",
        ] {
            assert!(legend.contains(part), "{part}: {legend}");
        }
        assert_eq!(lines(&m, FULL_HEIGHT - 1).len(), 2);
    }

    #[test]
    fn it_warns_near_the_threshold_and_says_when_compaction_is_due() {
        let near = view(200_000, 130_000, 40_000); // 151k of a 160k limit
        let m = model(ContextBarMode::Compact, near.clone());
        assert!(pressure(&near) >= WARN_AT);
        let head = &lines(&m, 40)[0];
        let limit = head
            .spans
            .iter()
            .find(|s| s.content == "160k")
            .expect("limit");
        assert_eq!(limit.style.fg, Some(m.theme.warning));
        let due = view(200_000, 150_000, 40_000);
        let m = model(ContextBarMode::Compact, due);
        assert!(text(&lines(&m, 40)[0]).contains("compacting next turn"));
    }

    #[test]
    fn auto_compact_off_has_no_tick_and_says_so() {
        let m = model(ContextBarMode::Compact, view(200_000, 10_000, 0));
        let rows = lines(&m, 40);
        assert!(text(&rows[0]).contains("auto-compact off"));
        assert!(!text(&rows[1]).contains('┃'));
    }

    #[test]
    fn it_stays_out_of_the_way_when_off_small_or_unknown() {
        let v = view(1_000_000, 1_000, 200_000);
        assert!(lines(&model(ContextBarMode::Off, v.clone()), 40).is_empty());
        assert!(lines(&model(ContextBarMode::Compact, v.clone()), MIN_HEIGHT - 1).is_empty());
        let mut m = model(ContextBarMode::Compact, v);
        m.context_meter = None;
        assert!(lines(&m, 40).is_empty());
    }

    #[test]
    fn rows_fit_the_width() {
        for width in [40u16, 60, 80, 100, 160] {
            let mut m = model(ContextBarMode::Full, view(1_000_000, 500_000, 200_000));
            m.width = width;
            for row in lines(&m, 40) {
                assert!(row.width() <= usize::from(width), "{width}: {}", text(&row));
            }
        }
    }
}
