//! Output-governor counters, policy and saved-output references.
//! Counters retain their denominators; stale entries are explicit without color.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_heading, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(status) = &model.governor else {
        return section_detail(
            width,
            th,
            "Output governor status is unavailable for this session.",
        );
    };
    let enabled = match status.enabled {
        Some(true) => "on",
        Some(false) => "off",
        None => "unknown",
    };
    let mut rows = section_heading(width, th, &format!("Governor: {enabled}"));
    for counter in &status.counters {
        rows.extend(section_heading(
            width,
            th,
            &format!("{}: {} {}", counter.verb, counter.number, counter.of),
        ));
        rows.extend(section_detail(width, th, &counter.note));
    }
    if status.stored.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No saved outputs listed for this session.",
        ));
    } else {
        rows.extend(section_heading(width, th, "Saved outputs"));
    }
    for entry in &status.stored {
        rows.extend(section_heading(
            width,
            th,
            &format!(
                "{} · {}{}",
                entry.id,
                entry.tool,
                if entry.stale { " · stale" } else { "" }
            ),
        ));
        rows.extend(section_detail(width, th, &entry.call));
        rows.extend(section_detail(width, th, &entry.size));
    }
    if status.stored.iter().any(|entry| !entry.stale) {
        rows.extend(section_detail(
            width,
            th,
            "Ask the agent to retrieve a listed output ID, optionally specifying a line range.",
        ));
    }
    for (label, value) in [
        ("Policy", &status.policy),
        ("Store", &status.store_dir),
        ("Session", &status.session_id),
        ("Since", &status.since),
        ("Outputs", &status.outputs_note),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    SheetChrome {
        header_right: model
            .governor
            .as_ref()
            .filter(|status| !status.session_id.is_empty())
            .map(|status| vec![span(format!("session {}", status.session_id), th.muted)])
            .unwrap_or_default(),
        status_third: model.governor.as_ref().map(|status| {
            vec![span(
                match status.enabled {
                    Some(true) => "governor on",
                    Some(false) => "governor off",
                    None => "governor status unknown",
                },
                th.muted,
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
        fixtures,
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
        m.governor = Some(fixtures::governor_sheet());
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
    fn counters_retain_their_number_denominator_purpose_and_note() {
        let m = model(120);
        let drawn = text(&m);
        let status = m.governor.as_ref().unwrap();
        for counter in &status.counters {
            for value in [&counter.number, &counter.of, &counter.verb, &counter.note] {
                assert!(drawn.contains(value), "{value}");
            }
        }
        assert!(drawn.contains(&status.policy) && drawn.contains(&status.store_dir));
        assert!(!drawn.contains("nothing is deleted"));
    }
    #[test]
    fn saved_outputs_have_full_ids_calls_sizes_and_explicit_stale_state() {
        let mut m = model(120);
        m.governor.as_mut().unwrap().stored[0].stale = true;
        let drawn = text(&m);
        for entry in &m.governor.as_ref().unwrap().stored {
            for value in [&entry.id, &entry.tool, &entry.call, &entry.size] {
                assert!(drawn.contains(value));
            }
        }
        assert!(drawn.contains("stale"));
        m.governor.as_mut().unwrap().enabled = None;
        assert!(text(&m).contains("Governor: unknown"));
    }
    #[test]
    fn missing_empty_and_narrow_statuses_are_bounded() {
        let mut m = model(80);
        m.governor.as_mut().unwrap().stored.clear();
        assert!(text(&m).contains("No saved outputs"));
        m.governor = None;
        assert!(text(&m).contains("unavailable"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
