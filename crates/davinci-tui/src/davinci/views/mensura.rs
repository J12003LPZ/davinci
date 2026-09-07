//! Context budget and policy advice. The view reports measurements and
//! recommendations; it does not expose actions the input owner cannot apply.

use crate::davinci::ui::{section_detail, section_heading, section_state};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let meta = &model.budget_meta;
    let mut rows = Vec::new();
    if !meta.in_use.is_empty() && !meta.window.is_empty() {
        rows.extend(section_heading(
            width,
            th,
            &format!("Context: {} of {}", meta.in_use, meta.window),
        ));
    }
    for (label, value) in [
        ("Headroom", &meta.headroom),
        ("Growth", &meta.rate),
        ("Policy", &meta.policy),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    if model.budget.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No per-role context budget data available.",
        ));
    }
    for item in &model.budget {
        let share = if item.fraction.is_finite() {
            format!(" · {:.0}%", item.fraction * 100.0)
        } else {
            String::new()
        };
        rows.extend(section_heading(
            width,
            th,
            &format!("{} · {} tokens{share}", item.role, item.tokens),
        ));
        if item.breach {
            rows.extend(section_state(width, th, State::Attention, &item.note));
        } else {
            rows.extend(section_detail(width, th, &item.note));
        }
    }
    if let Some(proposal) = &model.proposal {
        rows.extend(section_heading(width, th, "Policy recommendation"));
        rows.extend(section_detail(width, th, &proposal.summary));
        for (label, value) in [
            ("Recovers", &proposal.recovers),
            ("Keeps", &proposal.keeps),
            ("Cost", &proposal.cost),
        ] {
            if !value.is_empty() {
                rows.extend(section_detail(width, th, &format!("{label}: {value}")));
            }
        }
        rows.extend(section_detail(
            width,
            th,
            &format!(
                "Reversible: {}",
                if proposal.reversible { "yes" } else { "no" }
            ),
        ));
        if !proposal.actions.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                "Alternatives listed by the policy:",
            ));
            for (_, description) in &proposal.actions {
                rows.extend(section_detail(width, th, description));
            }
        }
        rows.extend(section_detail(
            width,
            th,
            "This review does not apply changes.",
        ));
    }
    for (label, value) in [
        ("Session spend", &meta.session_spend),
        ("Daily cap", &meta.daily_cap),
        ("History", &meta.history),
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
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn budgets_retain_role_counts_caps_and_textual_warnings() {
        let m = model(120);
        let drawn = text(&m);
        for row in &m.budget {
            assert!(
                drawn.contains(&row.role)
                    && drawn.contains(&row.tokens)
                    && drawn.contains(&row.note)
            );
        }
        assert!(
            drawn.contains(&m.budget_meta.session_spend)
                && drawn.contains(&m.budget_meta.daily_cap)
        );
        assert!(lines(&m).iter().any(|r| r.to_string().contains('!')
            && r.spans.iter().any(|s| s.style.fg == Some(m.theme.warning))));
    }
    #[test]
    fn advice_explains_its_effect_without_claiming_keyboard_actions() {
        let m = model(120);
        let proposal = m.proposal.as_ref().unwrap();
        let drawn = text(&m);
        for value in [
            &proposal.summary,
            &proposal.recovers,
            &proposal.keeps,
            &proposal.cost,
        ] {
            assert!(drawn.contains(value));
        }
        for (key, description) in &proposal.actions {
            assert!(!drawn.contains(&format!("[{key}]")));
            assert!(drawn.contains(description));
        }
        assert!(drawn.contains("This review does not apply changes"));
    }
    #[test]
    fn missing_invalid_and_narrow_budget_data_do_not_overflow() {
        let mut m = model(80);
        m.budget.clear();
        m.proposal = None;
        assert!(text(&m).contains("No per-role"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.budget[0].fraction = f64::NAN;
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
            assert!(!text(&m).contains("NaN"));
        }
    }
}
