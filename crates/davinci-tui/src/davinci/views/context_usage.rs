//! `/context`: the context window as a ten-by-ten grid of cells beside a
//! per-category legend, then the members of the categories that have them.
//!
//! Follows Claude Code's `/context` layout: `■` a full cell, `▪` a partly
//! used one, `□` free space and `▒` the autocompact buffer, which fills the
//! grid from the end. Claude Code draws `⛁ ⛀ ⛶ ⛝`, but neither Cascadia Mono
//! (Windows Terminal) nor Consolas (conhost) has them, so the terminal falls
//! back to a wider symbol font and the grid turns into overlapping boxes.
//! Every cell glyph here is in both fonts.
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
use crate::davinci::ui::{span, truncate_run};

pub const FULL: &str = "■";
pub const PARTIAL: &str = "▪";
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
pub const SECTION_ROWS: usize = 8;

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
    match kind {
        ContextKind::SystemPrompt => cc.prompt_border,
        ContextKind::SystemTools => cc.inactive,
        ContextKind::McpTools => cc.plan_mode,
        ContextKind::CustomAgents => cc.permission,
        ContextKind::MemoryFiles => cc.claude,
        ContextKind::Messages => cc.accept_edits,
    }
}

fn cell_span(cc: &Cc, cell: Cell) -> Span<'static> {
    match cell {
        Cell::Full(kind) => span(FULL, kind_color(cc, kind)),
        Cell::Partial(kind) => span(PARTIAL, kind_color(cc, kind)),
        Cell::Free => span(FREE, cc.subtle),
        Cell::Buffer => span(BUFFER, cc.inactive),
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
            span(format!("{FULL} "), kind_color(&cc, category.kind)),
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
            span(format!("{name}: "), theme.text),
            span(format!("{} tokens", format_tokens(tokens)), cc.inactive),
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
        assert!(rows[1].starts_with("     ■ ▪ "));
        assert!(rows[1].ends_with("claude-opus-5-5 · 40k/200k tokens (20%)"));
        assert!(rows[3].ends_with("Estimated usage by category"));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("■ System prompt: 3.1k tokens (1.6%)")));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("□ Free space: 127.6k (63.8%)")));
        assert!(rows
            .iter()
            .any(|row| row.ends_with("▒ Autocompact buffer: 32.4k tokens (16.2%)")));
        assert!(rows.iter().any(|row| row == "     Memory files"));
        assert!(rows.iter().any(|row| row == "     └ AGENTS.md: 900 tokens"));
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
        assert_eq!(items[0].as_str(), "     └ agent-79: 119 tokens");
        assert_eq!(
            items[SECTION_ROWS - 1].as_str(),
            "     └ agent-72: 112 tokens"
        );
        // The other 72 rows hold 40..=111 tokens.
        let rest: u64 = (40..=111).sum();
        assert_eq!(
            items[SECTION_ROWS].as_str(),
            format!("     └ 72 more: {} tokens", format_tokens(rest))
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
        assert!(!rows.iter().any(|row| row.contains(" more:")));
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
        assert!(names[0].starts_with("00:") && names[79].starts_with("79:"));
        assert!(!rows.iter().any(|row| row.contains(" more:")));
    }

    #[test]
    fn cell_glyphs_are_distinct_single_width_wgl4_shapes() {
        // WGL4 is the set every Windows console font carries. Font coverage
        // itself is checked against Cascadia Mono in tests/terminal_glyphs.rs.
        for glyph in [FULL, PARTIAL, FREE, BUFFER] {
            assert!(
                ["■", "▪", "□", "▫", "░", "▒", "▓", "█"].contains(&glyph),
                "{glyph} is not in the WGL4 set every Windows console font draws"
            );
            assert_eq!(unicode_width::UnicodeWidthStr::width(glyph), 1);
        }
        let distinct: std::collections::HashSet<_> =
            [FULL, PARTIAL, FREE, BUFFER].into_iter().collect();
        assert_eq!(distinct.len(), 4);
    }
}
