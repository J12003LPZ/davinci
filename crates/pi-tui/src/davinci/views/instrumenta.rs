//! `1d` — the command palette, an inset overlay over a dimmed transcript.
//!
//! Selection is marked by a 3-cell copper left bar *plus* a tinted row, so it
//! reads without color. The footer states the corpus (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/instrumenta.ex`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::davinci::model::{CorpusItem, Model};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{clip, pad, run_width, span, span_on, Surface};

/// The 3-cell copper bar that marks the selected row.
pub const SELECTION_BAR: &str = "▌  ";
const UNSELECTED_BAR: &str = "   ";

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset).saturating_sub(4);
    let hits = model.filtered_corpus();
    let selected = model.selection(hits.len());

    let mut body: Vec<Vec<Span<'static>>> = vec![query_row(model, inner, hits.len())];
    body.push(Vec::new());

    if hits.is_empty() {
        body.push(vec![span("no instrument matches that query", th.muted)]);
    } else {
        for (index, item) in hits.iter().enumerate() {
            body.push(row(th, inner, item, Some(index) == selected));
        }
    }

    body.push(Vec::new());
    body.push(vec![
        span("↑↓ move", th.border),
        span(" · ", th.border),
        span("enter run", th.border),
        span(" · ", th.border),
        span("tab complete", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]);
    body.push(vec![
        span("fuzzy: ", th.border),
        span("tools", th.muted),
        span(" · ", th.border),
        span("sessions", th.muted),
        span(" · ", th.border),
        span("files", th.muted),
        span(" · ", th.border),
        span("modes", th.muted),
    ]);

    Surface::new(width, th)
        .inset(inset)
        .title(vec![span("INSTRUMENTA", th.primary)])
        .right(vec![span("ctrl+p", th.border)])
        .rows(body)
        .lines()
}

fn query_row(model: &Model, inner: u16, hits: usize) -> Vec<Span<'static>> {
    let th = &model.theme;
    let caret = if model.blink() {
        Style::default().bg(th.primary).fg(th.background)
    } else {
        Style::default().bg(th.background).fg(th.background)
    };
    let left = vec![
        span(format!("{} ", glyph::PROMPT), th.primary),
        span(model.query.clone(), th.text),
        Span::styled(" ", caret),
    ];
    let right = vec![span(format!("{hits} of {}", model.corpus_total), th.border)];
    let gap = inner
        .saturating_sub(run_width(&left))
        .saturating_sub(run_width(&right))
        .max(1);
    let mut row = left;
    row.push(pad(gap, None));
    row.extend(right);
    row
}

fn row(theme: &Theme, inner: u16, item: &CorpusItem, selected: bool) -> Vec<Span<'static>> {
    let tint = if selected { Some(theme.surface) } else { None };
    let bar = if selected {
        span_on(SELECTION_BAR, theme.primary, tint)
    } else {
        span_on(UNSELECTED_BAR, theme.border, tint)
    };
    let name_color = if selected { theme.text } else { theme.muted };

    let name_column = (inner / 3).max(24);
    // The name is clipped to its column so a long path never squeezes the kind
    // off the row.
    let name = clip(
        &item.name,
        name_column.saturating_sub(SELECTION_BAR.len() as u16 + 1),
    );

    let left = vec![bar, span_on(name, name_color, tint)];
    let middle = vec![span_on(item.description.clone(), theme.muted, tint)];
    let right = vec![span_on(item.kind.clone(), theme.border, tint)];

    let name_pad = name_column.saturating_sub(run_width(&left)).max(1);
    let middle_pad = inner
        .saturating_sub(name_column)
        .saturating_sub(run_width(&middle))
        .saturating_sub(run_width(&right))
        .max(1);

    let mut row = left;
    row.push(pad(name_pad, tint));
    row.extend(middle);
    row.push(pad(middle_pad, tint));
    row.extend(right);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Overlay;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_overlay(Overlay::Instrumenta);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_palette_is_an_inset_surface_that_names_itself_and_its_key() {
        let m = model(100);
        let rows = lines(&m);
        let top = text(&rows[0]);
        assert!(top.trim_start().starts_with("╭─ INSTRUMENTA ─"), "{top}");
        assert!(top.ends_with("─ ctrl+p ─╮"), "{top}");
        assert!(top.starts_with("      "), "inset by six at 100: {top}");
        for row in &rows {
            assert_eq!(run_width(&row.spans), 100);
        }
    }

    #[test]
    fn the_query_row_states_how_much_of_the_corpus_is_left() {
        let mut m = model(100);
        m.type_char("git");
        let drawn = text(&lines(&m)[1]);
        assert!(drawn.contains("› git"), "{drawn}");
        assert!(drawn.contains(&format!("of {}", m.corpus_total)), "{drawn}");
    }

    #[test]
    fn selection_is_a_copper_bar_and_a_tint_not_color_alone() {
        let m = model(100);
        let rows = lines(&m);
        let selected = &rows[3];
        assert!(
            text(selected).contains(SELECTION_BAR),
            "{:?}",
            text(selected)
        );
        assert!(
            selected
                .spans
                .iter()
                .any(|span| span.style.bg == Some(m.theme.surface)),
            "the selected row is tinted"
        );
        let other = &rows[4];
        assert!(!text(other).contains('▌'));
        assert!(other.spans.iter().all(|span| span.style.bg.is_none()));
    }

    #[test]
    fn the_query_narrows_the_corpus() {
        let mut m = model(100);
        let all = m.filtered_corpus().len();

        // The mockup's corpus is what `git` already matched: tools, a session,
        // two files and a mode, `7 of 214`.
        m.type_char("git");
        assert_eq!(m.filtered_corpus().len(), all);
        let names: Vec<String> = m
            .filtered_corpus()
            .iter()
            .map(|item| item.name.clone())
            .collect();
        assert!(names.iter().any(|name| name == "/git status"), "{names:?}");
        assert!(
            names.iter().any(|name| name == ".gitignore"),
            "the corpus is files and sessions too: {names:?}"
        );

        m.query.clear();
        m.type_char("commit");
        let some = m.filtered_corpus().len();
        assert!(some > 0 && some < all, "{some} of {all}");
    }

    #[test]
    fn a_long_name_is_clipped_so_the_kind_still_reads() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        let row = rows
            .iter()
            .find(|row| row.contains("crates\\davinci-git"))
            .expect("the long file row");
        assert!(row.trim_end().ends_with("file │"), "{row}");
        assert!(row.contains("414 lines"), "{row}");
    }

    #[test]
    fn a_query_that_matches_nothing_says_so() {
        let mut m = model(100);
        m.type_char("zzzzzz");
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("no instrument matches that query")));
    }

    #[test]
    fn the_footer_states_the_exits_and_the_corpus() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("esc close")));
        assert!(rows
            .iter()
            .any(|row| row.contains("fuzzy: tools · sessions · files · modes")));
    }

    #[test]
    fn the_overlay_fills_the_window_below_eighty_columns() {
        let m = model(72);
        let rows = lines(&m);
        assert!(!text(&rows[0]).starts_with(' '), "no inset below 80");
        for row in &rows {
            assert_eq!(run_width(&row.spans), 72);
        }
    }

    #[test]
    fn moving_the_selection_wraps() {
        let mut m = model(100);
        let len = m.filtered_corpus().len();
        m.move_selection(-1);
        assert_eq!(m.selection(len), Some(len - 1));
        m.move_selection(1);
        assert_eq!(m.selection(len), Some(0));
    }
}
