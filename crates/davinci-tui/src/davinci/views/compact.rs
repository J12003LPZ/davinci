//! `/compact`: measured before/after results from the completed operation.
//! The runtime performs compaction before opening this view; there is no
//! second confirmation or alternative action hidden behind presentation keys.

use super::sheet::{Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(result) = &model.compaction else {
        return section_detail(width, th, "No compaction result. Use /compact from the conversation when there is context to summarize.");
    };
    let mut rows = section_detail(
        width,
        th,
        &format!(
            "Context: {} → {}",
            result.before_tokens, result.after_tokens
        ),
    );
    for (label, value) in [
        ("Before", &result.before_note),
        ("After", &result.after_note),
        ("Recovered", &result.recovers),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    for (label, values) in [("Kept", &result.kept), ("Summarized", &result.folded)] {
        if !values.is_empty() {
            rows.extend(section_detail(width, th, label));
            for value in values {
                rows.extend(section_detail(width, th, &format!("  {value}")));
            }
        }
    }
    for (label, value) in [
        ("Summary size", &result.note_cost),
        ("Summarizing call", &result.call_cost),
        ("Cache", &result.cache_cost),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows.extend(section_detail(width, th, &result.history));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    SheetChrome {
        status_third: Some(vec![span("compaction result", model.theme.muted)]),
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::Compaction,
        theme::{ColorDepth, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.compaction = Some(Compaction {
            before_tokens: "40k".into(),
            after_tokens: "8k".into(),
            recovers: "32k".into(),
            kept: vec!["recent messages".into()],
            folded: vec!["earlier messages".into()],
            call_cost: "not yet available".into(),
            history: "compacted once".into(),
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
    fn results_keep_measured_context_content_and_cost_information() {
        let drawn = text(&model(80));
        for value in [
            "40k → 8k",
            "32k",
            "recent messages",
            "earlier messages",
            "not yet available",
            "compacted once",
        ] {
            assert!(drawn.contains(value));
        }
        for value in [
            "compact now",
            "[enter]",
            "[e]",
            "[t]",
            "reversible",
            "pays a full",
        ] {
            assert!(!drawn.contains(value));
        }
    }
    #[test]
    fn empty_results_and_short_widths_are_honest_and_bounded() {
        let mut m = model(80);
        m.compaction = None;
        assert!(text(&m).contains("No compaction result"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
