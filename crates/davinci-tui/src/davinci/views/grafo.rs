//! `2a` — the code graph.
//!
//! The graph is drawn on a strict column grid: the parent connector column is
//! inherited by every child row and no vertical descends through label text.
//! Below it, an impact list; untested edges in warning (design.md §6).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/grafo.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::{GraphInk, Model};
use crate::davinci::theme::State;
use crate::davinci::ui::{blank, span, span_strong, spread, truncate_run, Surface, MEASURE};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 16);
    let meta = &model.graph_meta;

    let drawing: Vec<Vec<Span<'static>>> = model
        .graph
        .iter()
        .map(|row| {
            row.0
                .iter()
                .map(|(text, ink)| match ink {
                    GraphInk::Connector => span(text.clone(), th.border),
                    GraphInk::Name => span(text.clone(), th.muted),
                    GraphInk::Current => span_strong(text.clone(), th.primary, th),
                })
                .collect()
        })
        .collect();

    let mut rows = Surface::new(width, th)
        .title(vec![
            span("GRAFO", th.primary),
            span(" · ", th.border),
            span("DEPENDENCY STUDY", th.muted),
        ])
        .right(vec![
            span(format!("{} nodes", meta.nodes), th.muted),
            span(" · ", th.border),
            span(format!("{} edges", meta.edges), th.muted),
            span(" · ", th.border),
            span(format!("{} cycles", meta.cycles), th.success),
        ])
        .rows(drawing)
        .lines();

    rows.push(blank());
    rows.push(spread(
        width,
        vec![
            span("impact of ", th.text),
            span(meta.subject.clone(), th.secondary),
        ],
        vec![
            span("fan-in ", th.muted),
            span(meta.fan_in.clone(), th.text),
            span(" · ", th.border),
            span("fan-out ", th.muted),
            span(meta.fan_out.clone(), th.text),
            span(" · ", th.border),
            span("depth ", th.muted),
            span(meta.depth.clone(), th.text),
        ],
    ));
    rows.push(rule(width, model));

    for item in &model.impact {
        let symbol_color = if item.state == State::Active {
            th.text
        } else {
            th.muted
        };
        let sites_color = if item.untested { th.warning } else { th.muted };
        rows.push(spread(
            width,
            vec![
                span_strong(
                    format!("{}  ", item.state.glyph()),
                    th.state_color(item.state),
                    th,
                ),
                span(item.symbol.clone(), symbol_color),
            ],
            // Both columns are padded to a fixed width so distance and call
            // sites line up down the list however long a symbol is.
            vec![
                span(format!("{:<10}", item.distance), th.muted),
                span(format!("{:>16}", item.sites), sites_color),
            ],
        ));
    }

    rows.push(rule(width, model));
    let mut footer = vec![
        span(format!("{} tests touch this path", meta.tests), th.muted),
        span(" · ", th.border),
        span(format!("{} untested edges", meta.untested), th.warning),
    ];
    // Annotations are dropped below 100 columns (design.md §7).
    if model.decoration() {
        footer.push(span(" · ", th.border));
        footer.push(span(meta.freshness.clone(), th.muted));
    }
    rows.push(Line::from(footer));

    // The drawing is authored at a fixed grid; a narrow window cuts it at the
    // right edge rather than reflowing it, which would break the columns.
    rows.into_iter()
        .map(|row| Line::from(truncate_run(row.spans, width)))
        .collect()
}

fn rule(width: u16, model: &Model) -> Line<'static> {
    Line::from(vec![span("─".repeat(width as usize), model.theme.border)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;
    use unicode_width::UnicodeWidthStr;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        fixtures::dress(&mut model);
        model.toggle_screen(Screen::Grafo);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_header_carries_nodes_edges_and_cycles() {
        let top = text(&lines(&model(120))[0]);
        assert!(top.contains("╭─ GRAFO · DEPENDENCY STUDY ─"), "{top}");
        assert!(top.contains("412 nodes · 1207 edges · 0 cycles"), "{top}");
    }

    #[test]
    fn every_child_row_inherits_its_parent_connector_column() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m)[1..1 + m.graph.len()]
            .iter()
            .map(|row| text(row))
            .collect();

        // The `┬` on the first row is the column every following `│` and `└`
        // must sit in.
        let column = drawn[0].find('┬').expect("a branch on the first row");
        for row in &drawn[1..] {
            let mark = row
                .char_indices()
                .find(|(index, ch)| *index >= column && matches!(ch, '│' | '├' | '└'));
            if let Some((index, _)) = mark {
                assert!(
                    index >= column,
                    "connector at {index} left of the parent column {column}: {row}"
                );
            }
        }
    }

    #[test]
    fn no_vertical_descends_through_label_text() {
        let m = model(120);
        for row in lines(&m)[1..1 + m.graph.len()].iter() {
            for (text, ink) in m
                .graph
                .iter()
                .flat_map(|graph_row| graph_row.0.iter())
                .filter(|(_, ink)| *ink != GraphInk::Connector)
            {
                assert!(
                    !text.contains('│') && !text.contains('┬'),
                    "a label carries a connector: {text:?} ({ink:?})"
                );
            }
            let _ = row;
        }
    }

    #[test]
    fn the_node_in_hand_is_marked_with_its_glyph() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            drawn.iter().any(|row| row.contains("davinci-session ◉")),
            "the current node carries ◉"
        );
    }

    #[test]
    fn the_impact_list_marks_untested_edges_in_warning() {
        let m = model(120);
        let rows = lines(&m);
        let untested = rows
            .iter()
            .find(|row| text(row).contains("no test coverage"))
            .expect("the untested row");
        assert!(text(untested).starts_with("!  "), "{:?}", text(untested));
        assert!(untested
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.warning)));
    }

    #[test]
    fn every_impact_row_states_its_distance_and_call_sites() {
        let m = model(120);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for item in &m.impact {
            assert!(
                drawn.iter().any(|row| row.contains(&item.symbol)
                    && row.contains(&item.distance)
                    && row.contains(&item.sites)),
                "{} is missing its distance or call sites",
                item.symbol
            );
        }
    }

    #[test]
    fn the_study_never_grows_past_its_cap_and_nothing_overflows() {
        for width in [80u16, 100, 120, 160] {
            let cap = width.min(MEASURE + 16);
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= cap as usize,
                    "row wider than {cap} at terminal width {width}: {:?}",
                    text(&row)
                );
            }
        }
    }

    #[test]
    fn the_footer_reports_coverage_and_freshness() {
        let drawn: Vec<String> = lines(&model(120)).iter().map(text).collect();
        let footer = drawn.last().expect("footer");
        assert!(footer.contains("14 tests touch this path"), "{footer}");
        assert!(footer.contains("2 untested edges"), "{footer}");
        assert!(footer.contains("rust-analyzer"), "{footer}");
    }

    #[test]
    fn the_rules_span_the_study() {
        let m = model(120);
        let rows = lines(&m);
        let rules: Vec<&Line<'_>> = rows
            .iter()
            .filter(|row| text(row).chars().all(|ch| ch == '─') && !text(row).is_empty())
            .collect();
        assert_eq!(rules.len(), 2);
        for rule in rules {
            assert_eq!(run_width(&rule.spans), 120u16.min(MEASURE + 16));
        }
    }
}
