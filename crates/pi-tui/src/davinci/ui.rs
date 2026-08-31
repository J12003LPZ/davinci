//! Char-grid primitives.
//!
//! Everything here returns either a *run* (`Vec<Span>`) or a *row* (`Line`), so
//! callers can count rows exactly: the transcript tail-truncates like a
//! scrollback and the composer anchors to the bottom of the window at any
//! height. Surfaces are composed as rows rather than as ratatui `Block`s for
//! the same reason — a Studio box has to sit *inside* the transcript's row
//! list and still report its own height (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/ui.ex`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::theme::{glyph, State, Theme};

/// Prose never exceeds 74 columns, however wide the terminal (design.md §6).
pub const MEASURE: u16 = 74;

/// A run of text in one color.
pub fn span(content: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(content.into(), Style::default().fg(color))
}

/// A run of text in one color, on a tinted row.
pub fn span_on(
    content: impl Into<String>,
    color: Color,
    background: Option<Color>,
) -> Span<'static> {
    let mut style = Style::default().fg(color);
    if let Some(background) = background {
        style = style.bg(background);
    }
    Span::styled(content.into(), style)
}

/// A run of text carrying the theme's emphasis (bold under `NO_COLOR`, §9).
pub fn span_strong(content: impl Into<String>, color: Color, theme: &Theme) -> Span<'static> {
    Span::styled(
        content.into(),
        Style::default().fg(color).add_modifier(theme.emphasis),
    )
}

/// `n` cells of nothing, optionally tinted so a selected row reads as one band.
pub fn pad(n: u16, background: Option<Color>) -> Span<'static> {
    let mut style = Style::default();
    if let Some(background) = background {
        style = style.bg(background);
    }
    Span::styled(" ".repeat(n as usize), style)
}

/// Display width of a run, in terminal cells.
pub fn run_width(spans: &[Span<'_>]) -> u16 {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()) as u16)
        .sum()
}

/// An empty row.
pub fn blank() -> Line<'static> {
    Line::from(Vec::<Span<'static>>::new())
}

/// `n` empty rows.
pub fn blanks(n: usize) -> Vec<Line<'static>> {
    (0..n).map(|_| blank()).collect()
}

/// Left run, right run, flush to `width`.
pub fn spread(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>) -> Line<'static> {
    spread_on(width, left, right, None)
}

/// `spread`, with the gap tinted to match a selected row.
pub fn spread_on(
    width: u16,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    background: Option<Color>,
) -> Line<'static> {
    let gap = width
        .saturating_sub(run_width(&left))
        .saturating_sub(run_width(&right))
        .max(1);
    let mut spans = left;
    spans.push(pad(gap, background));
    spans.extend(right);
    Line::from(spans)
}

/// Centre a run in `width`.
pub fn center(width: u16, spans: Vec<Span<'static>>) -> Line<'static> {
    let lead = width.saturating_sub(run_width(&spans)) / 2;
    let mut row = vec![pad(lead, None)];
    row.extend(spans);
    Line::from(row)
}

/// Push a run right by `n` cells.
pub fn indent(n: u16, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![pad(n, None)];
    row.extend(spans);
    Line::from(row)
}

/// Truncate to `max` display cells, never mid-grapheme.
pub fn clip(text: &str, max: u16) -> String {
    if UnicodeWidthStr::width(text) <= max as usize {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max as usize {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// Wrap prose to the measure. Words longer than the measure are broken.
pub fn wrap(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        if current.is_empty() {
            while UnicodeWidthStr::width(word.as_str()) > width as usize {
                let head = clip(&word, width);
                word = word[head.len()..].to_string();
                lines.push(head);
            }
            current = word;
            continue;
        }
        let candidate_width =
            UnicodeWidthStr::width(current.as_str()) + 1 + UnicodeWidthStr::width(word.as_str());
        if candidate_width <= width as usize {
            current.push(' ');
            current.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut current));
            while UnicodeWidthStr::width(word.as_str()) > width as usize {
                let head = clip(&word, width);
                word = word[head.len()..].to_string();
                lines.push(head);
            }
            current = word;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Keep the last `n` rows — the transcript scrolls like a terminal.
pub fn tail(lines: Vec<Line<'static>>, n: usize) -> Vec<Line<'static>> {
    if lines.len() <= n {
        return lines;
    }
    lines[lines.len() - n..].to_vec()
}

/// Grow to exactly `n` rows so a column can be bottom-anchored.
pub fn pad_to(mut lines: Vec<Line<'static>>, n: usize) -> Vec<Line<'static>> {
    while lines.len() < n {
        lines.push(blank());
    }
    lines
}

/// A proportion meter: filled run, tip, empty run. Always exactly `width`
/// cells, so a number is never shown without its cap (design.md §9).
pub fn meter(fraction: f64, width: u16, theme: &Theme, color: Option<Color>) -> Vec<Span<'static>> {
    let color = color.unwrap_or(theme.primary);
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as u16).min(width);
    if filled == 0 {
        return vec![span(
            glyph::METER_EMPTY.repeat(width as usize),
            theme.border,
        )];
    }
    vec![
        span(glyph::METER_FILLED.repeat((filled - 1) as usize), color),
        span(glyph::METER_TIP, color),
        span(
            glyph::METER_EMPTY.repeat((width - filled) as usize),
            theme.border,
        ),
    ]
}

/// The `constructio III / V` tick meter (design.md §6).
pub fn ticks(done: usize, total: usize, cell_width: u16, theme: &Theme) -> Vec<Span<'static>> {
    if total == 0 {
        return vec![span(glyph::TICK.repeat(cell_width as usize), theme.border)];
    }
    let per = (cell_width as usize / total).max(1);
    vec![
        span(glyph::METER_FILLED.repeat(per * done), theme.primary),
        span(
            glyph::TICK.repeat(per * total.saturating_sub(done)),
            theme.border,
        ),
    ]
}

/// A bordered surface with its label notched into the top-left of the rule and
/// optional metadata notched into the top-right (design.md §3).
pub struct Surface {
    width: u16,
    inset: u16,
    border: Color,
    title: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    body: Vec<Vec<Span<'static>>>,
}

impl Surface {
    pub fn new(width: u16, theme: &Theme) -> Self {
        Self {
            width,
            inset: 0,
            border: theme.border,
            title: Vec::new(),
            right: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Draw the rule in something other than `border` — copper for the
    /// composer, warning for the governor proposal.
    pub fn border(mut self, color: Color) -> Self {
        self.border = color;
        self
    }

    /// Push the whole surface right, and narrow it by the same amount.
    pub fn inset(mut self, inset: u16) -> Self {
        self.inset = inset;
        self
    }

    pub fn title(mut self, title: Vec<Span<'static>>) -> Self {
        self.title = title;
        self
    }

    pub fn right(mut self, right: Vec<Span<'static>>) -> Self {
        self.right = right;
        self
    }

    pub fn row(mut self, row: Vec<Span<'static>>) -> Self {
        self.body.push(row);
        self
    }

    pub fn rows(mut self, rows: Vec<Vec<Span<'static>>>) -> Self {
        self.body.extend(rows);
        self
    }

    /// Row count, known before rendering.
    pub fn height(&self) -> usize {
        self.body.len() + 2
    }

    pub fn lines(self) -> Vec<Line<'static>> {
        let width = self.width.saturating_sub(self.inset).max(4);
        let inner = width - 4;
        let mut out = Vec::with_capacity(self.height());

        out.push(self.top_rule(width));
        for row in self.body {
            let used = run_width(&row);
            let mut spans = vec![span("│ ", self.border)];
            spans.extend(row);
            spans.push(pad(inner.saturating_sub(used), None));
            spans.push(span(" │", self.border));
            out.push(Line::from(spans));
        }
        out.push(Line::from(vec![span(
            format!("╰{}╯", "─".repeat((width - 2) as usize)),
            self.border,
        )]));

        if self.inset == 0 {
            out
        } else {
            out.into_iter()
                .map(|line| {
                    let mut spans = vec![pad(self.inset, None)];
                    spans.extend(line.spans);
                    Line::from(spans)
                })
                .collect()
        }
    }

    fn top_rule(&self, width: u16) -> Line<'static> {
        let mut left = if self.title.is_empty() {
            vec![span("╭─", self.border)]
        } else {
            let mut left = vec![span("╭─ ", self.border)];
            left.extend(self.title.iter().cloned());
            left.push(span(" ", self.border));
            left
        };
        let right = if self.right.is_empty() {
            vec![span("─╮", self.border)]
        } else {
            let mut right = vec![span("─ ", self.border)];
            right.extend(self.right.iter().cloned());
            right.push(span(" ─╮", self.border));
            right
        };
        let dashes = width
            .saturating_sub(run_width(&left))
            .saturating_sub(run_width(&right))
            .max(1);
        left.push(span("─".repeat(dashes as usize), self.border));
        left.extend(right);
        Line::from(left)
    }
}

/// A rule inside a surface, separating a header row from its list.
pub fn surface_rule(width: u16, theme: &Theme) -> Vec<Span<'static>> {
    vec![span(
        "─".repeat(width.saturating_sub(4) as usize),
        theme.border,
    )]
}

/// A plain horizontal rule with a mark at its centre (startup, `1a`).
pub fn hair_rule(width: u16, theme: &Theme, mark: &str) -> Line<'static> {
    let mark_width = UnicodeWidthStr::width(mark) as u16;
    let arm = width.saturating_sub(4 + mark_width).max(2) / 2;
    let dash = "─".repeat(arm as usize);
    Line::from(vec![
        span(format!("·{dash} "), theme.border),
        span(mark.to_string(), theme.muted),
        span(format!(" {dash}·"), theme.border),
    ])
}

/// `glyph  instrument · verb   target   duration` — one line, no box (§6).
pub fn tool_line(
    width: u16,
    theme: &Theme,
    state: State,
    instrument: &str,
    target: &str,
    duration: Option<&str>,
) -> Line<'static> {
    let body = width.saturating_sub(2).min(MEASURE + 4);
    let target_color = match state {
        State::Read | State::Search => theme.secondary,
        _ => theme.muted,
    };
    let left = vec![
        span_strong(
            format!("{} ", state.glyph()),
            theme.state_color(state),
            theme,
        ),
        span(instrument.to_string(), theme.muted),
        span(" · ", theme.border),
        span(target.to_string(), target_color),
    ];
    let right = match duration {
        Some(duration) => vec![span(duration.to_string(), theme.border)],
        None => Vec::new(),
    };
    let row = spread(body, left, right);
    indent(2, row.spans)
}

/// Tool detail — an error body or a diff hunk — indented two further (§3).
pub fn detail_line(theme: &Theme, text: &str) -> Line<'static> {
    indent(6, vec![span(text.to_string(), theme.muted)])
}

/// Whether a run carries the theme's emphasis; used by tests and by the
/// `NO_COLOR` audit.
pub fn is_strong(span: &Span<'_>) -> bool {
    span.style.add_modifier.contains(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;

    fn theme() -> Theme {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    }

    fn text_of(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn width_of(line: &Line<'_>) -> u16 {
        UnicodeWidthStr::width(text_of(line).as_str()) as u16
    }

    #[test]
    fn the_measure_is_seventy_four() {
        assert_eq!(MEASURE, 74);
    }

    #[test]
    fn prose_wraps_at_the_measure_however_wide_the_terminal() {
        let prose = "A request enters davinci-agent as a Turn, is planned, then dispatched \
                     to the provider adapter. Session state is written after every tool \
                     call, so an interrupt never loses the transcript.";
        for width in [MEASURE, 100, 160, 400] {
            let measure = MEASURE.min(width);
            for row in wrap(prose, measure) {
                assert!(
                    UnicodeWidthStr::width(row.as_str()) <= measure as usize,
                    "row {row:?} exceeds the measure at terminal width {width}"
                );
            }
        }
    }

    #[test]
    fn wrap_breaks_a_word_longer_than_the_measure() {
        let long = "crates/davinci-session/src/".to_string() + &"x".repeat(90);
        let rows = wrap(&long, 20);
        assert!(rows.len() > 1);
        for row in &rows {
            assert!(UnicodeWidthStr::width(row.as_str()) <= 20);
        }
        assert_eq!(rows.concat().replace(' ', ""), long.replace(' ', ""));
    }

    #[test]
    fn wrap_of_empty_prose_is_one_empty_row() {
        assert_eq!(wrap("", 74), vec![String::new()]);
        assert_eq!(wrap("anything", 0), vec![String::new()]);
    }

    #[test]
    fn clip_never_exceeds_the_cap() {
        assert_eq!(clip("store.rs", 40), "store.rs");
        assert_eq!(clip("crates/davinci-session", 6), "crates");
        assert_eq!(
            UnicodeWidthStr::width(clip("Δ".repeat(20).as_str(), 5).as_str()),
            5
        );
    }

    #[test]
    fn spread_fills_exactly_the_given_width() {
        let th = theme();
        let line = spread(
            80,
            vec![span("agent · main", th.primary)],
            vec![span("47k/200k", th.muted)],
        );
        assert_eq!(width_of(&line), 80);
    }

    #[test]
    fn spread_keeps_one_cell_of_gap_when_it_overflows() {
        let th = theme();
        let line = spread(
            10,
            vec![span("a very long left run", th.text)],
            vec![span("and a right one", th.text)],
        );
        assert!(text_of(&line).contains("left run and a right one"));
    }

    #[test]
    fn center_splits_the_slack_evenly() {
        let th = theme();
        let line = center(21, vec![span("DAVINCI", th.text)]);
        assert_eq!(text_of(&line), "       DAVINCI");
    }

    #[test]
    fn meter_is_always_exactly_its_width() {
        let th = theme();
        for width in [8u16, 12, 20, 24] {
            for step in 0..=20 {
                let fraction = step as f64 / 20.0;
                let cells = run_width(&meter(fraction, width, &th, None));
                assert_eq!(cells, width, "fraction {fraction} at width {width}");
            }
        }
    }

    #[test]
    fn meter_arithmetic_matches_the_status_bar() {
        let th = theme();
        // 47k of 200k, drawn in twelve cells (screen 1b).
        let spans = meter(47.0 / 200.0, 12, &th, None);
        let drawn: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(drawn, "━━╸─────────");
        assert_eq!(run_width(&spans), 12);
    }

    #[test]
    fn an_empty_meter_is_all_rule_and_a_full_one_ends_in_its_tip() {
        let th = theme();
        let empty: String = meter(0.0, 6, &th, None)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(empty, "──────");
        let full: String = meter(1.0, 6, &th, None)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(full, "━━━━━╸");
    }

    #[test]
    fn ticks_split_the_cell_run_between_done_and_queued() {
        let th = theme();
        let spans = ticks(3, 5, 20, &th);
        let drawn: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(drawn, "━━━━━━━━━━━━········");
        assert_eq!(run_width(&spans), 20);
        assert_eq!(spans[0].style.fg, Some(th.primary));
        assert_eq!(spans[1].style.fg, Some(th.border));
    }

    #[test]
    fn a_surface_is_two_rows_taller_than_its_body() {
        let th = theme();
        let surface = Surface::new(40, &th)
            .title(vec![span("STUDIO", th.primary)])
            .row(vec![span("surveyed workspace", th.text)])
            .row(vec![span("traced request pipeline", th.text)]);
        assert_eq!(surface.height(), 4);
        assert_eq!(surface.lines().len(), 4);
    }

    #[test]
    fn every_surface_row_is_exactly_the_surface_width() {
        let th = theme();
        for width in [40u16, 62, 80, 100] {
            let lines = Surface::new(width, &th)
                .title(vec![span("INSTRUMENTA", th.primary)])
                .right(vec![span("ctrl+p", th.border)])
                .row(vec![span("/git status", th.text)])
                .row(Vec::new())
                .lines();
            for line in &lines {
                assert_eq!(width_of(line), width, "row {:?}", text_of(line));
            }
        }
    }

    #[test]
    fn a_surface_notches_its_label_into_the_top_left_of_the_rule() {
        let th = theme();
        let lines = Surface::new(40, &th)
            .title(vec![span("STUDIO", th.primary)])
            .lines();
        let top = text_of(&lines[0]);
        assert!(top.starts_with("╭─ STUDIO ─"), "{top}");
        assert!(top.ends_with("─╮"), "{top}");
        assert_eq!(text_of(&lines[1]), format!("╰{}╯", "─".repeat(38)));
    }

    #[test]
    fn a_surface_notches_metadata_into_the_top_right_of_the_rule() {
        let th = theme();
        let lines = Surface::new(60, &th)
            .title(vec![span("GRAFO", th.primary)])
            .right(vec![span("0 cycles", th.success)])
            .lines();
        let top = text_of(&lines[0]);
        assert!(top.starts_with("╭─ GRAFO ─"), "{top}");
        assert!(top.ends_with("─ 0 cycles ─╮"), "{top}");
    }

    #[test]
    fn an_untitled_surface_still_rules_edge_to_edge() {
        let th = theme();
        let lines = Surface::new(30, &th)
            .row(vec![span("body", th.text)])
            .lines();
        assert_eq!(text_of(&lines[0]), format!("╭{}╮", "─".repeat(28)));
    }

    #[test]
    fn an_inset_surface_keeps_its_outer_width() {
        let th = theme();
        let lines = Surface::new(100, &th)
            .inset(6)
            .title(vec![span("MEMORIA", th.primary)])
            .row(vec![span("review-agent-runtime", th.text)])
            .lines();
        for line in &lines {
            assert_eq!(width_of(line), 100);
            assert!(text_of(line).starts_with("      "));
        }
    }

    #[test]
    fn a_surface_can_be_drawn_in_a_colour_other_than_border() {
        let th = theme();
        let lines = Surface::new(40, &th)
            .border(th.warning)
            .title(vec![span("GOVERNOR", th.warning)])
            .lines();
        assert_eq!(lines[0].spans[0].style.fg, Some(th.warning));
    }

    #[test]
    fn a_tool_call_is_one_line_indented_under_the_agent_mark() {
        let th = theme();
        let line = tool_line(
            100,
            &th,
            State::Done,
            "manus",
            "cargo check -p davinci-agent",
            Some("1.84s"),
        );
        let drawn = text_of(&line);
        assert!(drawn.starts_with("  ✓ manus · cargo check -p davinci-agent"));
        assert!(drawn.ends_with("1.84s"));
        assert_eq!(line.spans.iter().filter(|s| is_strong(s)).count(), 0);
    }

    #[test]
    fn a_read_targets_verdigris_and_a_failure_targets_muted() {
        let th = theme();
        let read = tool_line(100, &th, State::Read, "instrumenta", "lib.rs", None);
        assert_eq!(read.spans[4].style.fg, Some(th.secondary));
        let failed = tool_line(100, &th, State::Failed, "manus", "cargo test", None);
        assert_eq!(failed.spans[4].style.fg, Some(th.muted));
        assert_eq!(failed.spans[1].style.fg, Some(th.error));
    }

    #[test]
    fn tail_keeps_the_last_rows_and_pad_to_grows_to_height() {
        let th = theme();
        let rows: Vec<Line<'static>> = (0..10)
            .map(|i| Line::from(vec![span(i.to_string(), th.text)]))
            .collect();
        let kept = tail(rows.clone(), 3);
        assert_eq!(kept.len(), 3);
        assert_eq!(text_of(&kept[0]), "7");
        assert_eq!(tail(rows.clone(), 20).len(), 10);
        assert_eq!(pad_to(rows, 14).len(), 14);
    }

    #[test]
    fn a_hair_rule_carries_its_mark_at_the_centre() {
        let th = theme();
        let line = hair_rule(40, &th, "◦");
        let drawn = text_of(&line);
        assert!(drawn.starts_with('·') && drawn.ends_with('·'), "{drawn}");
        assert!(drawn.contains(" ◦ "), "{drawn}");
    }

    #[test]
    fn emphasis_is_only_applied_under_no_color() {
        let plain = theme();
        let no_color = Theme::da_vinci(ColorDepth::TrueColor, true);
        assert!(!is_strong(&span_strong("✓", plain.success, &plain)));
        assert!(is_strong(&span_strong("✓", no_color.success, &no_color)));
    }
}
