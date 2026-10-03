//! `/trust`: review the project's resources before opening the real decision.
//! This view does not infer the current trust decision or claim files are unread.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(project) = &model.project_trust else {
        return section_detail(width, th, "No project trust information available.");
    };
    let mut rows = section_detail(width, th, &project.path);
    rows.extend(section_detail(width, th, "Project resources can change prompts, configuration, and executable extensions. Review their origin before granting trust."));
    if project.files.is_empty() {
        rows.extend(section_detail(width, th, "No project resources listed."));
    }
    for file in &project.files {
        let mut label = section_detail(width, th, &file.path);
        for row in &mut label {
            for s in &mut row.spans {
                s.style.fg = Some(th.text);
            }
        }
        rows.extend(label);
        let mut risk = section_detail(width, th, &file.risk_label);
        if file.risk_label.contains("executes") || file.risk_label.contains("limits") {
            for row in &mut risk {
                for s in &mut row.spans {
                    s.style.fg = Some(th.warning);
                }
            }
        }
        rows.extend(risk);
        rows.extend(section_detail(width, th, &file.detail));
    }
    for (label, value) in [
        ("Trusted paths", &project.trusted),
        ("Ignored paths", &project.ignored),
        ("Decision store", &project.store),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows.extend(section_detail(width, th, "Enter opens the available trust choices. Escape closes this review without changing trust."));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    SheetChrome {
        header_right: model
            .project_trust
            .as_ref()
            .filter(|p| p.first_visit)
            .map(|_| vec![span("first visit", model.theme.muted)])
            .unwrap_or_default(),
        status_third: Some(vec![span("trust review", model.theme.muted)]),
        hints: vec![hint(&model.theme, "enter decide")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{ProjectTrustSheet, TrustFile},
        theme::{ColorDepth, State, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.project_trust = Some(ProjectTrustSheet {
            path: "example-project".into(),
            files: vec![TrustFile {
                path: ".pi/extensions/review.ts".into(),
                detail: "Workspace extension".into(),
                risk_label: "executes code".into(),
                state: State::Queued,
            }],
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
    fn resource_risk_and_actual_decision_entry_point_remain_readable() {
        let m = model(80);
        let drawn = text(&m);
        for value in [
            "example-project",
            ".pi/extensions/review.ts",
            "executes code",
            "Workspace extension",
            "Enter opens",
        ] {
            assert!(drawn.contains(value));
        }
        assert!(lines(&m)
            .iter()
            .any(|r| r.to_string().contains("executes code")
                && r.spans.iter().any(|s| s.style.fg == Some(m.theme.warning))));
        for value in [
            "Nothing here has been read",
            "[t]",
            "[o]",
            "[p]",
            "[n]",
            "no tools loaded",
        ] {
            assert!(!drawn.contains(value));
        }
        assert_eq!(chrome(&m).status_third.unwrap()[0].content, "trust review");
    }
    #[test]
    fn empty_and_unicode_trust_reviews_are_bounded() {
        let mut m = model(80);
        m.project_trust = None;
        assert!(text(&m).contains("No project trust information"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.project_trust.as_mut().unwrap().path = "项目/café/🦀".into();
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
