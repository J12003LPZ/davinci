//! Read-only implementation plan. Step state is supplied by the runtime;
//! no decorative art or unimplemented accept/edit controls compete with it.

use crate::davinci::ui::{section_detail, section_heading, section_state};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.plan.is_empty() {
        return section_detail(width, th, "No plan for this session yet.");
    }
    let completed = model
        .plan
        .iter()
        .filter(|step| step.state == State::Done)
        .count();
    let mut rows = section_heading(
        width,
        th,
        &format!("{completed} of {} steps complete", model.plan.len()),
    );
    for (index, step) in model.plan.iter().enumerate() {
        rows.extend(section_state(
            width,
            th,
            step.state,
            &format!("{}. {}", index + 1, step.verb),
        ));
        if let Some(target) = &step.target {
            rows.extend(section_detail(width, th, target));
        }
    }
    rows
}

/// Roman numerals remain available to callers that supply plan positions.
pub fn roman(value: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = value;
    let mut out = String::new();
    for (amount, numeral) in TABLE {
        while left >= amount {
            out.push_str(numeral);
            left -= amount;
        }
    }
    out
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
            24,
            false,
        );
        fixtures::dress(&mut m);
        m
    }
    #[test]
    fn the_plan_counts_completed_steps_not_the_current_cursor() {
        let m = model(120);
        let rows = lines(&m);
        let completed = m
            .plan
            .iter()
            .filter(|step| step.state == State::Done)
            .count();
        assert!(rows[0]
            .to_string()
            .contains(&format!("{completed} of {}", m.plan.len())));
        let text = rows
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for step in &m.plan {
            assert!(text.contains(&step.verb));
        }
        assert!(
            !text.contains("a accept")
                && !text.contains("e edit step")
                && !text.contains("parity first")
        );
        assert!(ui::focused_row(&rows).is_none());
    }
    #[test]
    fn roman_numerals_still_cover_all_previous_positions() {
        for (number, expected) in [
            (0, ""),
            (1, "I"),
            (3, "III"),
            (4, "IV"),
            (5, "V"),
            (9, "IX"),
            (14, "XIV"),
        ] {
            assert_eq!(roman(number), expected);
        }
    }
    #[test]
    fn empty_and_unicode_plans_fit_the_terminal() {
        let mut m = model(80);
        m.plan.clear();
        assert!(lines(&m)[0].to_string().contains("No plan"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.plan[0].verb = "Review 项目/café/🦀 very long step name".into();
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
