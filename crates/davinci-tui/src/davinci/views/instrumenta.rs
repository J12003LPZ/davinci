//! Searchable command palette. The existing corpus and matching logic own
//! ordering and selection; names take priority over descriptions and kind.

use super::sheet::hint;
use crate::davinci::model::Model;
pub use crate::davinci::ui::SELECTION_BAR;
use crate::davinci::ui::{self, section_detail, section_row, span, Surface};
use ratatui::{
    style::Style,
    text::{Line, Span},
};

pub fn lines(model: &Model, height: usize) -> Vec<Line<'static>> {
    ui::window_section(
        all_lines(model),
        height,
        2,
        model.overlay_offset,
        &model.theme,
    )
}

pub(crate) fn all_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = model.overlay_inset();
    let inner = model.width.saturating_sub(inset * 2).saturating_sub(4);
    let hits = model.filtered_corpus();
    let selected = model.selection(hits.len());
    let caret = if model.blink() {
        Style::default().fg(th.background).bg(th.primary)
    } else {
        Style::default()
    };
    let mut body = vec![Line::from(ui::truncate_run(
        vec![
            span("Search: ", th.muted),
            span(
                ui::clip_tail(&model.query, inner.saturating_sub(9)),
                th.text,
            ),
            Span::styled(" ", caret),
        ],
        inner,
    ))];
    if hits.is_empty() {
        body.extend(section_detail(
            inner,
            th,
            if model.corpus.is_empty() {
                "No commands or resources available."
            } else {
                "No matches. Change or clear the search."
            },
        ));
    }
    for (index, item) in hits.iter().enumerate() {
        let focused = Some(index) == selected;
        body.push(section_row(inner, th, focused, &item.name, &item.kind));
        if focused {
            if !body
                .last()
                .map(|row| row.to_string().contains(&item.name))
                .unwrap_or(false)
            {
                body.extend(section_detail(inner, th, &item.name));
            }
            body.extend(section_detail(inner, th, &item.description));
            if inner < 32 && !item.kind.is_empty() {
                body.extend(section_detail(inner, th, &format!("Type: {}", item.kind)));
            }
        }
    }
    body.push(ui::hint_row(
        inner,
        &[
            hint(th, "↑↓ move"),
            hint(
                th,
                if model.overlay_offset.is_some() {
                    "enter back"
                } else {
                    "enter select"
                },
            ),
        ],
        Some("esc close"),
        th,
    ));
    Surface::section(model.width, th)
        .inset(inset)
        .title(vec![span("Commands", th.primary)])
        .right(vec![span(
            format!(
                "{} of {}",
                hits.len(),
                model.corpus_total.max(model.corpus.len())
            ),
            th.muted,
        )])
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        model::{CorpusItem, Overlay},
        theme::{ColorDepth, Theme},
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress(&mut m);
        m.toggle_overlay(Overlay::Instrumenta);
        m
    }
    fn text(rows: &[Line<'_>]) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn search_is_real_and_preserves_the_query_and_available_count() {
        let mut m = model(80);
        let all = m.filtered_corpus().len();
        m.type_char("commit");
        assert!(!m.filtered_corpus().is_empty() && m.filtered_corpus().len() < all);
        let rows = lines(&m, 32);
        assert!(rows[0].to_string().contains("Commands") && rows[0].to_string().contains("of 214"));
        assert!(rows[1].to_string().contains("Search: commit"));
        assert!(!text(&rows).contains("tab complete"));
    }
    #[test]
    fn focus_expands_a_description_without_hiding_long_names_or_the_exit() {
        let mut m = model(40);
        m.corpus = vec![CorpusItem::new(
            "long-readable-command-name",
            "Full command description",
            "command",
        )];
        let rows = all_lines(&m);
        let drawn = text(&rows);
        assert!(
            drawn.contains("long-readable-command-name")
                && drawn.contains("Full command description")
        );
        assert!(drawn.contains("esc close"));
        assert!(rows[ui::focused_row(&rows).unwrap()]
            .spans
            .iter()
            .any(|s| s.style.fg == Some(m.theme.cc().permission)));
    }
    #[test]
    fn large_corpora_keep_query_title_selection_and_help_while_windowing() {
        let mut m = model(80);
        m.corpus = (0..200)
            .map(|i| CorpusItem::new(&format!("command-{i}"), "Description", "command"))
            .collect();
        m.corpus_total = 200;
        m.palette_index = 120;
        let rows = lines(&m, 12);
        let drawn = text(&rows);
        assert!(rows.len() <= 12);
        for value in [
            "Commands",
            "Search:",
            "command-120",
            "above",
            "below",
            "esc close",
        ] {
            assert!(drawn.contains(value), "{drawn}");
        }
        m.move_selection(80);
        assert_eq!(m.selection(200), Some(0));
        m.move_selection(-1);
        assert_eq!(m.selection(200), Some(199));
    }
    #[test]
    fn empty_results_and_long_unicode_queries_remain_bounded() {
        let mut m = model(80);
        m.type_char("zzzzzzzz");
        assert!(text(&lines(&m, 24)).contains("No matches"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.query = "项目 café 🦀 ".repeat(30);
            for row in lines(&m, 20) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
        assert!(ui::clip_tail("long query café 🦀", 10).ends_with("café 🦀"));
    }
}
