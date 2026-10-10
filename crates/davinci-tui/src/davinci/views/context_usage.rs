//! `/context`: the context window as a ten-by-ten grid of cells beside a
//! per-category legend, then the members of the categories that have them.
//!
//! Follows Claude Code's `/context` layout, with `□` free space and `▒` the
//! autocompact buffer, which fills the grid from the end. Each category has
//! its own glyph and its own hue, so two neighbours stay apart on a palette
//! with no colour at all and for readers who cannot tell two hues apart. A
//! partly used cell keeps its category's glyph in a dimmer ink. Claude Code
//! draws `⛁ ⛀ ⛶ ⛝`, but neither Cascadia Mono (Windows Terminal) nor
//! Consolas (conhost) has them, so the terminal falls back to a wider symbol
//! font and the grid turns into overlapping boxes. Every glyph here is in
//! both fonts.
//!
//! In the TUI each section shows its heaviest rows and folds the rest into a
//! count, so one long list (eighty `/agents`) no longer pushes the grid off a
//! 40-row screen. The cap is per section: three long sections together can
//! still outgrow a short terminal, and then the block scrolls like any other.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::transcript::ELBOW;
use crate::davinci::model::{ContextKind, ContextUsageView};
use crate::davinci::theme::{Cc, Theme};
use crate::davinci::ui::{mix, span, truncate_run};

pub const FREE: &str = "□";
pub const BUFFER: &str = "▒";

const COLUMNS: usize = 10;
const ROWS: usize = 10;
const CELLS: usize = COLUMNS * ROWS;
/// Lines up with the text after the elbow.
const INDENT: &str = "     ";
/// Below this the legend moves under the grid.
const SIDE_BY_SIDE: u16 = 72;
/// Rows a section shows in the TUI before folding the rest into a count.
pub const SECTION_ROWS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Full(ContextKind),
    Partial(ContextKind),
    Free,
    Buffer,
}

/// `1234` → `1.2k`, `45000` → `45k`, `999` → `999`.
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        return tokens.to_string();
    }
    let (value, unit) = if tokens >= 1_000_000 {
        (tokens as f64 / 1_000_000.0, "M")
    } else {
        (tokens as f64 / 1000.0, "k")
    };
    let text = format!("{value:.1}");
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

fn percent(tokens: u64, window: u64) -> f64 {
    if window == 0 {
        0.0
    } else {
        tokens as f64 * 100.0 / window as f64
    }
}

/// The hundred cells in reading order: each used category takes its share
/// (at least one cell, the last of them partial when the share is not
/// whole), the buffer takes the end, and free space takes what is between.
pub fn cells(view: &ContextUsageView) -> Vec<Cell> {
    let window = view.window.max(1) as f64;
    let mut grid = Vec::with_capacity(CELLS);
    for category in view.categories.iter().filter(|c| c.tokens > 0) {
        let exact = category.tokens as f64 * CELLS as f64 / window;
        let full = exact.floor() as usize;
        let partial = exact - exact.floor() > 0.0 || full == 0;
        grid.extend(std::iter::repeat_n(Cell::Full(category.kind), full));
        if partial {
            grid.push(Cell::Partial(category.kind));
        }
    }
    grid.truncate(CELLS);
    let buffer = if view.buffer == 0 {
        0
    } else {
        ((view.buffer as f64 * CELLS as f64 / window).round() as usize).max(1)
    };
    let buffer = buffer.min(CELLS - grid.len());
    let free = CELLS - grid.len() - buffer;
    grid.extend(std::iter::repeat_n(Cell::Free, free));
    grid.extend(std::iter::repeat_n(Cell::Buffer, buffer));
    grid
}

pub(crate) fn kind_color(cc: &Cc, kind: ContextKind) -> Color {
    cc.category[match kind {
        ContextKind::SystemPrompt => 0,
        ContextKind::SystemTools => 1,
        ContextKind::McpTools => 2,
        ContextKind::CustomAgents => 3,
        ContextKind::MemoryFiles => 4,
        ContextKind::Messages => 5,
    }]
}

/// One glyph per category, every one single-width and in WGL4. None of them
/// is `□` (free space) or `▒` (the buffer).
pub const fn kind_glyph(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::SystemPrompt => "■",
        ContextKind::SystemTools => "▲",
        ContextKind::McpTools => "♦",
        ContextKind::CustomAgents => "●",
        ContextKind::MemoryFiles => "▼",
        ContextKind::Messages => "█",
    }
}

fn cell_span(cc: &Cc, cell: Cell) -> Span<'static> {
    match cell {
        Cell::Full(kind) => span(kind_glyph(kind), kind_color(cc, kind)),
        Cell::Partial(kind) => partial_span(cc, kind),
        Cell::Free => span(FREE, cc.subtle),
        Cell::Buffer => span(BUFFER, cc.inactive),
    }
}

/// A partly used cell: its category's glyph in a dimmer ink. `mix` only blends
/// two RGB inks and otherwise returns one of them whole, which on a 256-colour
/// or basic palette would draw every partial cell in `subtle` grey. There the
/// category's own ink is kept and the terminal's faint attribute dims it.
fn partial_span(cc: &Cc, kind: ContextKind) -> Span<'static> {
    let ink = kind_color(cc, kind);
    match (ink, cc.subtle) {
        (Color::Rgb(..), Color::Rgb(..)) => span(kind_glyph(kind), mix(ink, cc.subtle, 0.55)),
        _ => Span::styled(
            kind_glyph(kind),
            Style::default().fg(ink).add_modifier(Modifier::DIM),
        ),
    }
}

fn bold(content: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(
        content.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn legend(theme: &Theme, view: &ContextUsageView) -> Vec<Vec<Span<'static>>> {
    let cc = theme.cc();
    let used = view.used();
    let mut rows = vec![
        vec![
            span(format!("{} · ", view.model), theme.text),
            span(
                format!(
                    "{}/{} tokens ({:.0}%)",
                    format_tokens(used),
                    format_tokens(view.window),
                    percent(used, view.window)
                ),
                theme.text,
            ),
        ],
        Vec::new(),
        vec![Span::styled(
            "Estimated usage by category",
            Style::default()
                .fg(cc.inactive)
                .add_modifier(Modifier::ITALIC),
        )],
    ];
    for category in &view.categories {
        rows.push(vec![
            span(
                format!("{} ", kind_glyph(category.kind)),
                kind_color(&cc, category.kind),
            ),
            span(format!("{}: ", category.label), theme.text),
            span(
                format!(
                    "{} tokens ({:.1}%)",
                    format_tokens(category.tokens),
                    percent(category.tokens, view.window)
                ),
                cc.inactive,
            ),
        ]);
    }
    rows.push(vec![
        span(format!("{FREE} "), cc.subtle),
        span("Free space: ", theme.text),
        span(
            format!(
                "{} ({:.1}%)",
                format_tokens(view.free),
                percent(view.free, view.window)
            ),
            cc.inactive,
        ),
    ]);
    if view.buffer > 0 {
        rows.push(vec![
            span(format!("{BUFFER} "), cc.inactive),
            span("Autocompact buffer: ", theme.text),
            span(
                format!(
                    "{} tokens ({:.1}%)",
                    format_tokens(view.buffer),
                    percent(view.buffer, view.window)
                ),
                cc.inactive,
            ),
        ]);
    }
    rows
}

pub fn lines(theme: &Theme, view: &ContextUsageView, width: u16) -> Vec<Line<'static>> {
    render(theme, view, width, Some(SECTION_ROWS))
}

fn item_row(cc: &Cc, theme: &Theme, name: String, tokens: u64, width: u16) -> Line<'static> {
    Line::from(truncate_run(
        vec![
            Span::raw(INDENT),
            span("└ ", cc.inactive),
            span(format!("{name} "), theme.text),
            span(format_tokens(tokens), cc.inactive),
        ],
        width,
    ))
}

/// `limit` caps each section's rows, heaviest first; `None` lists them all.
fn render(
    theme: &Theme,
    view: &ContextUsageView,
    width: u16,
    limit: Option<usize>,
) -> Vec<Line<'static>> {
    let cc = theme.cc();
    let mut out = vec![Line::from(vec![
        span(ELBOW, cc.inactive),
        bold("Context Usage", theme.text),
    ])];

    let grid = cells(view);
    let grid_rows: Vec<Vec<Span<'static>>> = grid
        .chunks(COLUMNS)
        .map(|row| {
            let mut spans = Vec::with_capacity(COLUMNS * 2);
            for (index, cell) in row.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(cell_span(&cc, *cell));
            }
            spans
        })
        .collect();
    let legend = legend(theme, view);
    let grid_width = COLUMNS * 2 - 1;

    if width >= SIDE_BY_SIDE {
        for index in 0..grid_rows.len().max(legend.len()) {
            let mut spans = vec![Span::raw(INDENT)];
            match grid_rows.get(index) {
                Some(row) => spans.extend(row.iter().cloned()),
                None => spans.push(Span::raw(" ".repeat(grid_width))),
            }
            if let Some(row) = legend.get(index).filter(|row| !row.is_empty()) {
                spans.push(Span::raw("   "));
                spans.extend(row.iter().cloned());
            }
            out.push(Line::from(truncate_run(spans, width)));
        }
    } else {
        for row in grid_rows {
            let mut spans = vec![Span::raw(INDENT)];
            spans.extend(row);
            out.push(Line::from(truncate_run(spans, width)));
        }
        out.push(Line::default());
        for row in legend {
            let mut spans = vec![Span::raw(INDENT)];
            spans.extend(row);
            out.push(Line::from(truncate_run(spans, width)));
        }
    }

    for section in view.sections.iter().filter(|s| !s.items.is_empty()) {
        out.push(Line::default());
        let mut heading = vec![Span::raw(INDENT), bold(section.title.clone(), theme.text)];
        if let Some(command) = &section.command {
            heading.push(span(format!(" · {command}"), cc.inactive));
        }
        out.push(Line::from(truncate_run(heading, width)));
        let mut items: Vec<&(String, u64)> = section.items.iter().collect();
        if limit.is_some() {
            // Heaviest first, so the rows kept are the ones worth seeing.
            items.sort_by(|a, b| b.1.cmp(&a.1));
        }
        // Folding a single row into "1 more" saves nothing.
        let shown = match limit {
            Some(limit) if items.len() > limit + 1 => limit,
            _ => items.len(),
        };
        for (name, tokens) in &items[..shown] {
            out.push(item_row(&cc, theme, name.clone(), *tokens, width));
        }
        let rest = &items[shown..];
        if !rest.is_empty() {
            let tokens = rest.iter().map(|(_, tokens)| tokens).sum();
            out.push(item_row(
                &cc,
                theme,
                format!("{} more", rest.len()),
                tokens,
                width,
            ));
        }
    }
    out
}

/// The same block as text, every row listed in reported order, for print
/// mode, RPC and the legacy chrome, which all write to normal scrollback.
pub fn plain_lines(view: &ContextUsageView, width: u16) -> Vec<String> {
    let theme = Theme::da_vinci(crate::davinci::theme::ColorDepth::TrueColor, true);
    render(&theme, view, width, None)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .replace('\u{a0}', " ")
                .trim_end()
                .to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{ContextCategory, ContextSection};

    fn view() -> ContextUsageView {
        ContextUsageView {
            model: "claude-opus-5-5".into(),
            window: 200_000,
            categories: vec![
                ContextCategory {
                    kind: ContextKind::SystemPrompt,
                    label: "System prompt".into(),
                    tokens: 3_100,
                },
                ContextCategory {
                    kind: ContextKind::SystemTools,
                    label: "System tools".into(),
                    tokens: 11_400,
                },
                ContextCategory {
                    kind: ContextKind::MemoryFiles,
                    label: "Memory files".into(),
                    tokens: 900,
                },
                ContextCategory {
                    kind: ContextKind::Messages,
                    label: "Messages".into(),
                    tokens: 24_600,
                },
            ],
            free: 127_616,
            buffer: 32_384,
            sections: vec![ContextSection {
                title: "Memory files".into(),
                command: None,
                items: vec![("AGENTS.md".into(), 900)],
            }],
        }
    }

    #[test]
    fn tokens_read_like_claude_code() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1k");
        assert_eq!(format_tokens(3_140), "3.1k");
        assert_eq!(format_tokens(200_000), "200k");
        assert_eq!(format_tokens(1_000_000), "1M");
    }

    #[test]
    fn grid_is_a_hundred_cells_with_the_buffer_at_the_end() {
        let grid = cells(&view());
        assert_eq!(grid.len(), CELLS);
        assert_eq!(grid[0], Cell::Full(ContextKind::SystemPrompt));
        // 3.1k of 200k is 1.55 cells: one full, one partial.
        assert_eq!(grid[1], Cell::Partial(ContextKind::SystemPrompt));
        // A category under one cell still shows.
        assert!(grid.contains(&Cell::Partial(ContextKind::MemoryFiles)));
        assert_eq!(grid.iter().filter(|c| **c == Cell::Buffer).count(), 16);
        assert_eq!(grid[CELLS - 1], Cell::Buffer);
        assert!(grid.contains(&Cell::Free));
    }

    #[test]
    fn full_window_never_overflows_the_grid() {
        let mut full = view();
        full.categories[3].tokens = 400_000;
        full.free = 0;
        assert_eq!(cells(&full).len(), CELLS);
    }

    #[test]
    fn plain_block_has_the_header_grid_legend_and_sections() {
        let rows = plain_lines(&view(), 120);
        assert_eq!(rows[0], "  ⎿  Context Usage");
        assert!(rows[1].starts_with("     ■ ■ "));
        assert!(rows[1].ends_with("claude-opus-5-5 · 40k/200k tokens (20%)"));
        assert!(rows[3].ends_with("Estimated usage by category"));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("■ System prompt: 3.1k tokens (1.6%)")));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("▲ System tools: 11.4k tokens (5.7%)")));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("□ Free space: 127.6k (63.8%)")));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("▒ Autocompact buffer: 32.4k tokens (16.2%)")));
        assert!(rows.iter().any(|row| row == "     Memory files"));
        assert!(rows.iter().any(|row| row == "     └ AGENTS.md 900"));
    }

    #[test]
    fn narrow_terminals_put_the_legend_under_the_grid() {
        let rows = plain_lines(&view(), 40);
        assert!(rows[1].trim_start().starts_with('■'));
        assert!(!rows[1].contains("tokens"));
        assert!(rows.iter().all(|row| row.chars().count() <= 40));
    }

    fn many_agents() -> ContextUsageView {
        let mut view = view();
        view.sections = vec![ContextSection {
            title: "Custom agents".into(),
            command: Some("/agents".into()),
            items: (0..80)
                .map(|index| (format!("agent-{index:02}"), 40 + index as u64))
                .collect(),
        }];
        view
    }

    fn text(view: &ContextUsageView, width: u16) -> Vec<String> {
        let theme = Theme::da_vinci(crate::davinci::theme::ColorDepth::TrueColor, true);
        lines(&theme, view, width)
            .iter()
            .map(|line| {
                line.to_string()
                    .replace('\u{a0}', " ")
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_long_section_shows_its_heaviest_rows_and_folds_the_rest() {
        let rows = text(&many_agents(), 120);
        let items: Vec<&String> = rows.iter().filter(|row| row.contains("└ ")).collect();
        assert_eq!(items.len(), SECTION_ROWS + 1);
        assert_eq!(items[0].as_str(), "     └ agent-79 119");
        let last_shown = 80 - SECTION_ROWS;
        assert_eq!(
            items[SECTION_ROWS - 1].as_str(),
            format!("     └ agent-{last_shown} {}", 40 + last_shown)
        );
        // The rest hold 40..40+last_shown tokens each.
        let rest: u64 = (40..40 + last_shown as u64).sum();
        assert_eq!(
            items[SECTION_ROWS].as_str(),
            format!("     └ {} more {}", 80 - SECTION_ROWS, format_tokens(rest))
        );
        // The whole block fits one 40-row screen, grid first.
        assert!(rows.len() < 40, "{} rows", rows.len());
        assert!(rows[1].starts_with("     ■"));
    }

    #[test]
    fn a_section_one_over_the_limit_is_listed_whole() {
        let mut view = many_agents();
        view.sections[0].items.truncate(SECTION_ROWS + 1);
        let rows = text(&view, 120);
        assert!(!rows.iter().any(|row| row.contains(" more ")));
        assert_eq!(
            rows.iter().filter(|row| row.contains("└ ")).count(),
            SECTION_ROWS + 1
        );
    }

    #[test]
    fn print_mode_still_lists_every_row() {
        let rows = plain_lines(&many_agents(), 120);
        let names: Vec<&str> = rows
            .iter()
            .filter_map(|row| row.strip_prefix("     └ agent-"))
            .collect();
        assert_eq!(names.len(), 80);
        // Print mode keeps the order the agent reported.
        assert!(names[0].starts_with("00 ") && names[79].starts_with("79 "));
        assert!(!rows.iter().any(|row| row.contains(" more ")));
    }

    const CATEGORIES: [ContextKind; 6] = [
        ContextKind::SystemPrompt,
        ContextKind::SystemTools,
        ContextKind::McpTools,
        ContextKind::CustomAgents,
        ContextKind::MemoryFiles,
        ContextKind::Messages,
    ];

    #[test]
    fn cell_glyphs_are_distinct_single_width_wgl4_shapes() {
        // WGL4 is the set every Windows console font carries. Font coverage
        // itself is checked against Cascadia Mono in tests/terminal_glyphs.rs.
        let mut glyphs: Vec<&str> = CATEGORIES.iter().map(|kind| kind_glyph(*kind)).collect();
        glyphs.extend([FREE, BUFFER]);
        for glyph in &glyphs {
            assert!(
                ["■", "□", "▪", "▫", "▲", "▼", "♦", "●", "░", "▒", "▓", "█"].contains(glyph),
                "{glyph} is not in the WGL4 set every Windows console font draws"
            );
            assert_eq!(unicode_width::UnicodeWidthStr::width(*glyph), 1);
        }
        let distinct: std::collections::HashSet<_> = glyphs.iter().collect();
        assert_eq!(distinct.len(), glyphs.len(), "{glyphs:?}");
    }

    #[test]
    fn every_category_has_its_own_ink_in_every_colour_theme() {
        use crate::davinci::theme::ColorDepth;
        for depth in [
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Basic,
        ] {
            for name in ["dark", "light", "vox"] {
                let theme = Theme::da_vinci(depth, false).with_name(name);
                let cc = theme.cc();
                let inks: Vec<Color> = CATEGORIES
                    .iter()
                    .map(|kind| kind_color(&cc, *kind))
                    .collect();
                for (a, first) in inks.iter().enumerate() {
                    for (b, second) in inks.iter().enumerate().skip(a + 1) {
                        assert_ne!(
                            first, second,
                            "{depth:?} {name}: {:?} and {:?} share an ink",
                            CATEGORIES[a], CATEGORIES[b]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_partial_cell_keeps_its_category_and_differs_from_a_full_one() {
        use crate::davinci::theme::ColorDepth;
        for depth in [
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Basic,
        ] {
            for name in ["dark", "light", "vox"] {
                let cc = Theme::da_vinci(depth, false).with_name(name).cc();
                let partials: Vec<Style> = CATEGORIES
                    .iter()
                    .map(|kind| cell_span(&cc, Cell::Partial(*kind)).style)
                    .collect();
                for (index, kind) in CATEGORIES.iter().enumerate() {
                    let full = cell_span(&cc, Cell::Full(*kind));
                    let partial = cell_span(&cc, Cell::Partial(*kind));
                    assert_eq!(partial.content, full.content, "{depth:?} {name} {kind:?}");
                    assert_ne!(partial.style, full.style, "{depth:?} {name} {kind:?}");
                    assert_ne!(
                        partial.style.fg,
                        Some(cc.subtle),
                        "{depth:?} {name}: a partial {kind:?} lost its ink"
                    );
                    for other in &partials[index + 1..] {
                        assert_ne!(&partial.style, other, "{depth:?} {name} {kind:?}");
                    }
                }
            }
        }
        // With no colour at all a partial cell is still drawn faint.
        let cc = Theme::da_vinci(ColorDepth::TrueColor, true).cc();
        let partial = cell_span(&cc, Cell::Partial(ContextKind::Messages));
        let full = cell_span(&cc, Cell::Full(ContextKind::Messages));
        assert_ne!(partial.style, full.style);
    }

    #[test]
    fn the_grid_draws_each_category_with_its_own_glyph() {
        let rows = plain_lines(&view(), 120);
        let grid: String = rows[1..=10]
            .iter()
            .map(|row| row.trim_start().chars().take(19).collect::<String>())
            .collect();
        for glyph in ["■", "▲", "▼"] {
            assert!(grid.contains(glyph), "{glyph} missing from {grid}");
        }
    }
}
