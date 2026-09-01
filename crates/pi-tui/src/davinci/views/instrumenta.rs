//! `1d` — the command palette, an inset overlay over a dimmed transcript.
//!
//! Selection is marked by a 3-cell copper left bar *plus* a tinted row, so it
//! reads without color. The footer states the corpus (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/instrumenta.ex`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::davinci::model::{CorpusItem, Model};
use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{
    clip_ellipsis, pad, run_width, span, span_on, spread, surface_rule, Surface,
};

/// The 3-cell copper bar that marks the selected row.
pub const SELECTION_BAR: &str = "▌  ";
const UNSELECTED_BAR: &str = "   ";

pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let width = model.width;
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let hits = model.filtered_corpus();
    let selected = model.selection(hits.len());

    // The query is ruled off from the results, as the results are from the
    // footer (`1d`).
    let mut body: Vec<Vec<Span<'static>>> = vec![query_row(model, inner, hits.len())];
    body.push(surface_rule(width.saturating_sub(inset * 2), th));

    if hits.is_empty() {
        body.push(vec![span("no instrument matches that query", th.muted)]);
    } else {
        // The palette keeps its head: the title, the query row and the footer
        // never scroll off however long the corpus is. A corpus taller than
        // the window is folded around the selection, and what is folded is
        // counted either side — the same rule every long sheet follows.
        // Six rows of the height are the surface's own: two borders, the
        // query, two rules and the footer.
        let avail = height.saturating_sub(6).max(3);
        let total = hits.len();
        if total <= avail {
            for (index, item) in hits.iter().enumerate() {
                body.push(row(th, inner, item, Some(index) == selected));
            }
        } else {
            let window = avail.saturating_sub(2).max(1);
            let anchor = selected.unwrap_or(0);
            let start = anchor
                .saturating_sub(window / 2)
                .min(total.saturating_sub(window));
            let end = start + window;
            if start > 0 {
                body.push(vec![span(format!("… {start} above"), th.border)]);
            }
            for (index, item) in hits.iter().enumerate().skip(start).take(window) {
                body.push(row(th, inner, item, Some(index) == selected));
            }
            if end < total {
                body.push(vec![span(format!("… {} below", total - end), th.border)]);
            }
        }
    }

    body.push(surface_rule(width.saturating_sub(inset * 2), th));
    // One footer row: the keys left, the corpus right (`1d`). When the row
    // cannot hold both, `tab complete` gives way before the exit does (§9).
    let keys = |with_tab: bool| {
        let mut keys = vec![
            span("↑↓ move", th.border),
            span(" │ ", th.border),
            span("enter run", th.border),
            span(" │ ", th.border),
        ];
        if with_tab {
            keys.push(span("tab complete", th.border));
            keys.push(span(" │ ", th.border));
        }
        keys.push(span("esc close", th.border));
        keys
    };
    let corpus_note = vec![
        span("fuzzy: ", th.border),
        span("tools", th.muted),
        span(" · ", th.border),
        span("sessions", th.muted),
        span(" · ", th.border),
        span("files", th.muted),
        span(" · ", th.border),
        span("modes", th.muted),
    ];
    let full = keys(true);
    let footer = if run_width(&full) + run_width(&corpus_note) < inner {
        spread(inner, full, corpus_note)
    } else {
        spread(inner, keys(false), corpus_note)
    };
    body.push(footer.spans);

    Surface::new(width, th)
        .inset(inset)
        .border(th.primary)
        .title(vec![span("INSTRUMENTA", th.primary)])
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

    // Wide enough for the corpus's longest path, so a file row is legible in
    // full (`1d` lists `crates\davinci-git\src\lib.rs` unclipped).
    let name_column = ((inner * 2) / 5).clamp(24, 34);
    let right = vec![span_on(item.kind.clone(), theme.border, tint)];
    let kind_width = run_width(&right);
    let middle_column = inner
        .saturating_sub(name_column)
        .saturating_sub(kind_width)
        .saturating_sub(2);

    // A short description leaves its slack to the name, so a session id or a
    // long path is read in full whenever the row can afford it; a name the row
    // truly cannot hold is cut with a visible ellipsis, never a bare cut that
    // reads as the whole name.
    // The bar is three cells wide on screen, whatever its byte length.
    let description_width = UnicodeWidthStr::width(item.description.as_str()) as u16;
    let spare = middle_column.saturating_sub(description_width.saturating_add(2));
    let name = clip_ellipsis(
        &item.name,
        name_column.saturating_sub(4).saturating_add(spare),
    );
    let left = vec![bar, span_on(name, name_color, tint)];

    // The description is clipped to what is left between the name and the
    // kind. A real tool description is a sentence, and an unclipped one pushed
    // the kind off its own row — `… supporting context to` with no `tool`
    // after it.
    let name_pad = name_column.saturating_sub(run_width(&left)).max(1);
    let middle_room = inner
        .saturating_sub(run_width(&left))
        .saturating_sub(name_pad)
        .saturating_sub(kind_width)
        .saturating_sub(1);
    let middle = vec![span_on(
        clip_ellipsis(&item.description, middle_room),
        theme.muted,
        tint,
    )];

    let middle_pad = inner
        .saturating_sub(run_width(&left))
        .saturating_sub(name_pad)
        .saturating_sub(run_width(&middle))
        .saturating_sub(kind_width)
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
    fn the_palette_is_an_inset_copper_surface_that_names_itself() {
        let m = model(100);
        let rows = lines(&m, 44);
        let top = text(&rows[0]);
        assert!(top.trim_start().starts_with("╭─ INSTRUMENTA ─"), "{top}");
        assert!(top.starts_with("      "), "inset by six at 100: {top}");
        assert!(top.ends_with("      "), "inset from both edges: {top:?}");
        assert_eq!(rows[0].spans[1].style.fg, Some(m.theme.primary));
        for row in &rows {
            assert_eq!(run_width(&row.spans), 100);
        }
    }

    #[test]
    fn the_query_row_states_how_much_of_the_corpus_is_left() {
        let mut m = model(100);
        m.type_char("git");
        let drawn = text(&lines(&m, 44)[1]);
        assert!(drawn.contains("› git"), "{drawn}");
        assert!(drawn.contains(&format!("of {}", m.corpus_total)), "{drawn}");
    }

    #[test]
    fn selection_is_a_copper_bar_and_a_tint_not_color_alone() {
        let m = model(100);
        let rows = lines(&m, 44);
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
        let rows: Vec<String> = lines(&m, 44).iter().map(text).collect();
        let row = rows
            .iter()
            .find(|row| row.contains("crates\\davinci-git"))
            .expect("the long file row");
        assert!(row.trim_end().ends_with("file │"), "{row}");
        assert!(row.contains("414 lines"), "{row}");
    }

    #[test]
    fn a_long_description_is_clipped_so_the_kind_still_reads() {
        // A real tool description is a sentence. Unclipped, it pushed the kind
        // off the row: `… supporting context to` with no `tool` after it.
        let mut m = model(100);
        m.corpus = vec![CorpusItem::new(
            "memory_search",
            "Search durable vector and lexical memory for supporting context to hand the model",
            "tool",
        )];
        m.corpus_total = 1;
        let rows: Vec<String> = lines(&m, 44).iter().map(text).collect();
        let row = rows
            .iter()
            .find(|row| row.contains("memory_search"))
            .expect("the tool row");
        assert!(row.trim_end().ends_with("tool │"), "{row}");
        assert!(row.contains("Search durable vector"), "{row}");
        for line in lines(&m, 44) {
            assert_eq!(run_width(&line.spans), 100);
        }
    }

    #[test]
    fn a_query_that_matches_nothing_says_so() {
        let mut m = model(100);
        m.type_char("zzzzzz");
        let drawn: Vec<String> = lines(&m, 44).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("no instrument matches that query")));
    }

    #[test]
    fn the_footer_states_the_exits_and_the_corpus_on_one_row() {
        let m = model(100);
        let rows: Vec<String> = lines(&m, 44).iter().map(text).collect();
        let footer = rows
            .iter()
            .find(|row| row.contains("esc close"))
            .expect("the footer");
        assert!(footer.contains("↑↓ move │ enter run"), "{footer}");
        assert!(
            footer.contains("fuzzy: tools · sessions · files · modes"),
            "{footer}"
        );

        // Wider, the full key run fits too.
        let m = model(120);
        let rows: Vec<String> = lines(&m, 44).iter().map(text).collect();
        let footer = rows
            .iter()
            .find(|row| row.contains("esc close"))
            .expect("the footer");
        assert!(footer.contains("tab complete"), "{footer}");
    }

    #[test]
    fn the_overlay_fills_the_window_below_eighty_columns() {
        let m = model(72);
        let rows = lines(&m, 44);
        assert!(!text(&rows[0]).starts_with(' '), "no inset below 80");
        for row in &rows {
            assert_eq!(run_width(&row.spans), 72);
        }
    }

    #[test]
    fn a_tall_corpus_is_folded_so_the_head_never_scrolls_off() {
        let mut m = model(100);
        m.corpus = (0..200)
            .map(|i| CorpusItem::new(&format!("tool_{i}"), "does the thing", "tool"))
            .collect();
        m.corpus_total = 200;
        let rows = lines(&m, 20);
        assert!(rows.len() <= 20, "{} rows for a height of 20", rows.len());
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(drawn[0].contains("INSTRUMENTA"), "the title survives");
        assert!(
            drawn.iter().any(|row| row.contains("of 200")),
            "the query row survives: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("below")),
            "the fold is counted: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("esc close")),
            "the footer survives"
        );
    }

    #[test]
    fn the_fold_follows_the_selection() {
        let mut m = model(100);
        m.corpus = (0..200)
            .map(|i| CorpusItem::new(&format!("tool_{i}"), "does the thing", "tool"))
            .collect();
        m.corpus_total = 200;
        for _ in 0..120 {
            m.move_selection(1);
        }
        let drawn: Vec<String> = lines(&m, 20).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("tool_120")),
            "the selected row is in the window: {drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("above"))
                && drawn.iter().any(|row| row.contains("below")),
            "counted both sides mid-list: {drawn:?}"
        );
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
