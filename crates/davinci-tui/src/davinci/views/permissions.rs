//! `/permissions`: the active mode and rules grouped by kind.
//! Enter still applies a mode or removes a rule through the existing policy code.

use ratatui::text::Line;

use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_row, span};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.permission_rows.is_empty() {
        return section_detail(width, th, "No permission policy loaded.");
    }
    let selected = model.permission_index % model.permission_rows.len();
    let mut rows = Vec::new();
    let mut previous_kind = "";
    for (index, row) in model.permission_rows.iter().enumerate() {
        if row.kind != previous_kind {
            rows.extend(section_detail(
                width,
                th,
                if row.kind == "mode" { "Modes" } else { "Rules" },
            ));
            previous_kind = &row.kind;
        }
        let value = if row.current {
            "current"
        } else if row.kind == "rule" {
            &row.detail
        } else {
            ""
        };
        let always_approve = row.kind == "mode" && row.key == "always-approve";
        let mut mode_row = section_row(width, th, index == selected, &row.label, value);
        if always_approve {
            for span in &mut mode_row.spans {
                span.style.fg = Some(th.warning);
                span.style = span.style.add_modifier(ratatui::style::Modifier::BOLD);
            }
        }
        rows.push(mode_row);
        if always_approve {
            let mut warning = section_detail(width, th, "! no prompts; deny rules still apply.");
            for span in warning.iter_mut().flat_map(|line| &mut line.spans) {
                span.style.fg = Some(th.warning);
            }
            rows.extend(warning);
        }
        if index == selected {
            if row.current {
                rows.extend(section_detail(width, th, "Current mode for this session"));
            }
            rows.extend(section_detail(width, th, &row.detail));
            if row.kind == "rule" {
                rows.extend(section_detail(width, th, &format!("Rule: {}", row.key)));
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Enter removes this rule from {} permissions.", row.source),
                ));
            }
        }
    }
    rows.extend(section_detail(
        width,
        th,
        "Deny rules take precedence over allow rules.",
    ));
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let mode = model
        .permission_rows
        .iter()
        .find(|r| r.kind == "mode" && r.current)
        .map(|r| r.label.clone());
    let rules = model
        .permission_rows
        .iter()
        .filter(|r| r.kind == "rule")
        .count();
    SheetChrome {
        header_right: facts(
            th,
            vec![
                mode.as_ref()
                    .map(|mode| vec![span(format!("mode {mode}"), th.muted)])
                    .unwrap_or_default(),
                vec![span(
                    format!("{rules} rule{}", if rules == 1 { "" } else { "s" }),
                    th.muted,
                )],
            ],
        ),
        status_third: mode.map(|mode| vec![span(mode, th.muted)]),
        hints: vec![hint(th, "↑↓ move"), hint(th, "enter apply")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::PermissionRow,
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
        m.permission_rows = [
            ("ask", "Manual", "Read tools run; edits ask"),
            (
                "edits",
                "Accept Edits",
                "Workspace edits run; commands may ask",
            ),
            (
                "read-only",
                "Plan Mode",
                "Explore without mutating the workspace",
            ),
            (
                "auto",
                "Auto Mode",
                "Routine safe actions run; risky actions ask",
            ),
            (
                "always-approve",
                "Always Approve",
                "Permitted actions run without approval prompts",
            ),
        ]
        .into_iter()
        .map(|(key, label, detail)| PermissionRow {
            label: label.into(),
            detail: detail.into(),
            current: key == "ask",
            kind: "mode".into(),
            key: key.into(),
            source: String::new(),
        })
        .collect();
        m.permission_rows.push(PermissionRow {
            label: "bash(git *)".into(),
            detail: "allow · user".into(),
            current: false,
            kind: "rule".into(),
            key: "bash(git *)".into(),
            source: "user".into(),
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
    fn mode_and_focus_are_distinct_and_rule_removal_names_its_source() {
        let mut m = model(80);
        m.permission_index = 5;
        let rows = lines(&m);
        assert!(rows[ui::focused_row(&rows).unwrap()]
            .to_string()
            .contains("bash(git *)"));
        assert!(rows
            .iter()
            .any(|r| r.to_string().contains("Manual") && r.to_string().contains("current")));
        for expected in [
            "Modes",
            "Rules",
            "allow · user",
            "Enter removes this rule from user permissions",
            "Deny rules take precedence",
        ] {
            assert!(text(&m).contains(expected), "{expected}");
        }
        assert_eq!(chrome(&m).composer, Composer::Hidden);
    }

    #[test]
    fn always_approve_row_warns_about_no_prompts_even_when_not_selected() {
        for width in [20, 32, 40, 80] {
            let mut m = model(width);
            m.permission_index = 0; // Always Approve remains unselected.
            let rows = lines(&m);
            let warning = rows
                .iter()
                .find(|row| row.to_string().contains("no prompts"))
                .expect("unselected Always Approve must disclose no prompts");
            assert!(warning
                .spans
                .iter()
                .any(|span| span.style.fg == Some(m.theme.warning)));
            assert!(rows.iter().all(|row| ui::run_width(&row.spans) <= width));
        }
    }

    #[test]
    fn empty_and_narrow_policies_do_not_overflow() {
        let mut m = model(80);
        m.permission_rows.clear();
        assert!(text(&m).contains("No permission policy"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.permission_rows[5].label = "bash(检查 café 🦀 *)".into();
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
