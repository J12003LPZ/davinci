//! `/thinking`: supported levels, current state, and runtime-supplied mappings.
//! Cursor movement previews a level; only the existing confirmation flow applies it.

use ratatui::text::Line;

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_row, span};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.thinking_rows.is_empty() {
        return section_detail(width, th, "No thinking levels available for this model.");
    }
    let selected = model.thinking_index % model.thinking_rows.len();
    let mut rows = Vec::new();
    for (index, level) in model.thinking_rows.iter().enumerate() {
        let current = level.level == model.thinking_level;
        let label = if current {
            format!("{} (current)", level.level)
        } else {
            level.level.clone()
        };
        let value = if level.warn {
            format!("! {}", level.budget)
        } else {
            level.budget.clone()
        };
        rows.push(section_row(width, th, index == selected, &label, &value));
        if index == selected {
            rows.extend(section_detail(
                width,
                th,
                &format!("Budget: {}", level.budget),
            ));
            let mut mapping = section_detail(width, th, &format!("Mapping: {}", level.maps_to));
            if level.warn {
                for row in &mut mapping {
                    for s in &mut row.spans {
                        s.style.fg = Some(th.warning);
                    }
                }
            }
            rows.extend(mapping);
        }
    }
    if !model.facts.thinking_reserve.is_empty() {
        rows.extend(section_detail(width, th, &model.facts.thinking_reserve));
    }
    if !model.facts.thinking_last_turn.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            &format!("Last turn: {}", model.facts.thinking_last_turn),
        ));
    }
    if model.facts.thinking_output_share > 0.0 {
        rows.extend(section_detail(
            width,
            th,
            &format!(
                "Thinking: {:.0}% of session output tokens",
                model.facts.thinking_output_share * 100.0
            ),
        ));
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    SheetChrome {
        header_right: vec![span(model.model_name.clone(), th.muted)],
        status_third: (!model.thinking_level.is_empty())
            .then(|| vec![span(format!("thinking {}", model.thinking_level), th.muted)]),
        hints: vec![hint(th, "↑↓ move"), hint(th, "enter select")],
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
        fixtures::dress_screen(&mut m, "3c");
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
    fn focus_and_applied_level_have_distinct_meanings() {
        let mut m = model(80);
        m.thinking_level = "low".into();
        m.thinking_index = m
            .thinking_rows
            .iter()
            .position(|r| r.level == "high")
            .unwrap();
        assert!(text(&m).contains("low (current)"));
        let rows = lines(&m);
        assert!(rows[ui::focused_row(&rows).unwrap()]
            .to_string()
            .contains("high"));
        assert_eq!(chrome(&m).status_third.unwrap()[0].content, "thinking low");
    }

    #[test]
    fn mapping_and_warning_come_from_the_focused_runtime_row() {
        let mut m = model(80);
        m.thinking_index = 0;
        m.thinking_rows[0].maps_to = "! custom provider budget exceeds window".into();
        m.thinking_rows[0].warn = true;
        let rows = lines(&m);
        let warning = rows
            .iter()
            .find(|r| r.to_string().contains("custom provider"))
            .unwrap();
        assert!(warning
            .spans
            .iter()
            .any(|s| s.style.fg == Some(m.theme.warning)));
        assert!(!text(&m).contains("SONNET → GPT"));
        assert!(!text(&m).contains("under budget"));
    }

    #[test]
    fn every_level_has_a_budget_at_normal_width_and_only_focus_has_mapping_details() {
        let m = model(80);
        let rows = lines(&m);
        for level in &m.thinking_rows {
            assert!(rows
                .iter()
                .any(|r| r.to_string().contains(&level.level)
                    && r.to_string().contains(&level.budget)));
        }
        assert_eq!(
            rows.iter()
                .filter(|r| r.to_string().contains("Mapping:"))
                .count(),
            1
        );
    }

    #[test]
    fn empty_and_narrow_levels_have_no_box_or_overflow() {
        let mut m = model(80);
        m.thinking_rows.clear();
        assert!(text(&m).contains("No thinking levels"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let m = model(width);
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
            assert!(!text(&m).contains('╭'));
        }
    }
}
