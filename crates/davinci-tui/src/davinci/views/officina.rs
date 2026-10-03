//! `/reload`: operation results and the resources actually reported as loaded.
//! Full errors and paths wrap; no synthetic load timing or "always on" status.

use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::ui::{section_detail, section_heading, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(workshop) = &model.workshop else {
        return section_detail(
            width,
            th,
            "No reload result available. Use /reload from the conversation.",
        );
    };
    let mut rows = Vec::new();
    for (state, message, duration, detail) in &workshop.reload {
        let label = if duration.is_empty() {
            message.clone()
        } else {
            format!("{message} · {duration}")
        };
        rows.extend(section_state(width, th, *state, &label));
        if let Some(detail) = detail {
            let mut detail_rows = section_detail(width, th, detail);
            if *state == State::Failed {
                for row in &mut detail_rows {
                    for span in &mut row.spans {
                        span.style.fg = Some(th.error);
                    }
                }
            }
            rows.extend(detail_rows);
        }
    }
    for (title, extensions) in [
        ("Native extensions", &workshop.native),
        ("JavaScript extensions", &workshop.javascript),
    ] {
        rows.extend(section_heading(width, th, title));
        if extensions.is_empty() {
            rows.extend(section_detail(width, th, "None reported."));
        }
        for (state, name, detail) in extensions {
            rows.extend(section_state(width, th, *state, name));
            rows.extend(section_detail(width, th, detail));
        }
    }
    for (label, value) in [
        ("Node", &workshop.node),
        ("Node details", &workshop.node_note),
        ("Node elapsed", &workshop.node_elapsed),
        ("Tool schema", &workshop.schema),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    if !workshop.tools.is_empty() {
        rows.extend(section_heading(width, th, "Tool inventory"));
    }
    for (label, count, fraction, note) in &workshop.tools {
        let share = if fraction.is_finite() {
            format!(" · {:.0}%", fraction * 100.0)
        } else {
            String::new()
        };
        rows.extend(section_heading(
            width,
            th,
            &format!("{label}: {count}{share}"),
        ));
        rows.extend(section_detail(width, th, note));
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let facts_ = &model.facts;
    let failures = model.workshop.as_ref().map(|workshop| {
        let listed = workshop
            .native
            .iter()
            .chain(&workshop.javascript)
            .filter(|(state, _, _)| *state == State::Failed)
            .count();
        listed.max(
            workshop
                .reload
                .iter()
                .filter(|(state, _, _, _)| *state == State::Failed)
                .count(),
        )
    });
    SheetChrome {
        header_right: facts(
            th,
            [
                (facts_.tool_count > 0)
                    .then(|| vec![span(format!("{} tools", facts_.tool_count), th.muted)]),
                (facts_.command_count > 0)
                    .then(|| vec![span(format!("{} commands", facts_.command_count), th.muted)]),
                (facts_.tool_schema_tokens > 0).then(|| {
                    vec![span(
                        format!("{} schema tokens", facts_.tool_schema_tokens),
                        th.muted,
                    )]
                }),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
        status_third: failures.map(|count| {
            vec![span(
                format!("{count} reported failures"),
                if count > 0 { th.error } else { th.muted },
            )]
        }),
        hints: vec![hint(th, "↑↓ scroll")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::WorkshopSheet,
        theme::{ColorDepth, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        m.workshop = Some(WorkshopSheet {
            reload: vec![
                (State::Done, "bindings loaded".into(), "3ms".into(), None),
                (
                    State::Failed,
                    "deploy.js failed".into(),
                    "318ms".into(),
                    Some("TypeError: hook is not a function\n3 tools were not registered".into()),
                ),
            ],
            native: vec![(State::Done, "vector-memory".into(), "4 tools".into())],
            javascript: vec![(State::Failed, "deploy.js".into(), "failed".into())],
            node: "node fixture".into(),
            ..Default::default()
        });
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
    fn reload_results_keep_actual_timings_errors_and_loaded_resources() {
        let m = model(120);
        let drawn = text(&m);
        for value in [
            "bindings loaded",
            "3ms",
            "318ms",
            "TypeError: hook is not a function",
            "3 tools were not registered",
            "vector-memory",
            "4 tools",
            "node fixture",
        ] {
            assert!(drawn.contains(value));
        }
        assert!(!drawn.contains("0ms") && !drawn.contains("ALWAYS ON"));
        assert!(lines(&m).iter().any(|r| r.to_string().contains("TypeError")
            && r.spans.iter().any(|s| s.style.fg == Some(m.theme.error))));
        assert_eq!(
            chrome(&m).status_third.unwrap()[0].content,
            "1 reported failures"
        );
    }
    #[test]
    fn unavailable_empty_and_narrow_reload_results_are_bounded() {
        let mut m = model(80);
        m.workshop = Some(WorkshopSheet::default());
        assert!(text(&m).contains("None reported"));
        m.workshop = None;
        assert!(text(&m).contains("No reload result"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
