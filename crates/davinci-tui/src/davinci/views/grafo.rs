//! Read-only dependency study. The supplied graph keeps its connector grid;
//! full symbol names and impact details remain available beneath the drawing.
//! An unavailable live symbol index is not replaced with fabricated data.

use crate::davinci::ui::{self, section_detail, section_heading, section_state, span};
use crate::davinci::{
    model::{GraphInk, Model},
    theme::State,
};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.graph.is_empty() && model.impact.is_empty() {
        return section_detail(
            width,
            th,
            "Dependency study unavailable: no symbol-index data was supplied for this session.",
        );
    }
    let meta = &model.graph_meta;
    let facts = [
        ("nodes", &meta.nodes),
        ("edges", &meta.edges),
        ("cycles", &meta.cycles),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(label, value)| format!("{value} {label}"))
    .collect::<Vec<_>>()
    .join(" · ");
    let mut rows = section_detail(width, th, &facts);
    for row in &model.graph {
        let mut spans = vec![span("   ", th.muted)];
        spans.extend(row.0.iter().map(|(text, ink)| {
            span(
                text.clone(),
                match ink {
                    GraphInk::Connector => th.border,
                    GraphInk::Name => th.muted,
                    GraphInk::Current => th.text,
                },
            )
        }));
        // Reflowing an authored graph would detach its connectors from nodes.
        rows.push(Line::from(ui::truncate_run(spans, width)));
    }
    if !meta.subject.is_empty() {
        rows.extend(section_heading(
            width,
            th,
            &format!("Impact: {}", meta.subject),
        ));
    }
    let facts = [
        ("Fan-in", &meta.fan_in),
        ("Fan-out", &meta.fan_out),
        ("Depth", &meta.depth),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(label, value)| format!("{label}: {value}"))
    .collect::<Vec<_>>()
    .join(" · ");
    rows.extend(section_detail(width, th, &facts));
    for item in &model.impact {
        rows.extend(section_state(
            width,
            th,
            if item.untested {
                State::Attention
            } else {
                item.state
            },
            &item.symbol,
        ));
        rows.extend(section_detail(
            width,
            th,
            &format!("{} · {}", item.distance, item.sites),
        ));
    }
    for (label, value) in [
        ("Tests touching this path", &meta.tests),
        ("Untested edges", &meta.untested),
        ("Index freshness", &meta.freshness),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        fixtures::dress(&mut m);
        m
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn metadata_impact_and_freshness_are_real_and_do_not_disappear_at_narrow_widths() {
        let m = model(120);
        let text = text(&m);
        for value in [
            &m.graph_meta.subject,
            &m.graph_meta.freshness,
            &m.graph_meta.nodes,
            &m.graph_meta.edges,
        ] {
            assert!(text.contains(value));
        }
        for item in &m.impact {
            assert!(
                text.contains(&item.symbol)
                    && text.contains(&item.distance)
                    && text.contains(&item.sites)
            );
        }
        assert!(lines(&m)
            .iter()
            .any(|r| r.spans.iter().any(|s| s.style.fg == Some(m.theme.warning))));
    }
    #[test]
    fn graph_connectors_keep_the_supplied_grid_without_inventing_edges() {
        let m = model(160);
        let rows = lines(&m);
        for graph_row in &m.graph {
            let expected = graph_row
                .0
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>();
            assert!(rows
                .iter()
                .any(|row| row.to_string() == format!("   {expected}")));
        }
        for (text, _) in m
            .graph
            .iter()
            .flat_map(|r| &r.0)
            .filter(|(_, ink)| *ink != GraphInk::Connector)
        {
            assert!(!text.contains('│') && !text.contains('┬'));
        }
    }
    #[test]
    fn unavailable_and_narrow_studies_do_not_fabricate_an_index() {
        let mut m = model(80);
        m.graph.clear();
        m.impact.clear();
        assert!(text(&m).contains("no symbol-index data"));
        assert!(!text(&m).contains("nodes"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
