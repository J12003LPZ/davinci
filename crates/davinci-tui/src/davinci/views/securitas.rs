//! Security report review: reported progress, findings and evidence.
//! A finding's severity and the keyboard cursor are independent. Network use,
//! validation guarantees and report persistence are not inferred by the view.

use super::sheet::{hint, status_meter, Composer, SheetChrome};
use crate::davinci::model::{Model, Severity};
use crate::davinci::ui::{section_detail, section_heading, section_row, span};
use ratatui::text::Line;

fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
        Severity::Dismissed => "dismissed",
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(scan) = &model.security else {
        return section_detail(
            width,
            th,
            "No security scan available. Ask the agent to run a scan from the conversation.",
        );
    };
    let mut rows = Vec::new();
    if !scan.state.is_empty() {
        rows.extend(section_heading(
            width,
            th,
            &format!("Status: {}", scan.state),
        ));
    }
    if !scan.id.is_empty() {
        rows.extend(section_detail(width, th, &format!("Scan: {}", scan.id)));
    }
    if scan.candidates > 0 {
        rows.extend(section_detail(
            width,
            th,
            &format!(
                "Validated: {} of {} candidates",
                scan.validated, scan.candidates
            ),
        ));
    }
    for (label, value) in [
        ("Files", &scan.files),
        ("Skipped", &scan.skipped),
        ("Read", &scan.bytes),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    if !scan.severities.is_empty() {
        let tally = scan
            .severities
            .iter()
            .map(|(label, count, _)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(" · ");
        rows.extend(section_detail(width, th, &tally));
    }
    if scan.dismissed > 0 {
        rows.extend(section_detail(
            width,
            th,
            &format!("Dismissed candidates: {}", scan.dismissed),
        ));
    }
    if scan.findings.is_empty() {
        rows.extend(section_detail(width, th, "No findings reported. This does not establish that the project is free of vulnerabilities."));
    }
    let selected = model.security_index % scan.findings.len().max(1);
    for (index, finding) in scan.findings.iter().enumerate() {
        let focused = index == selected;
        let severity = severity_word(finding.severity);
        rows.push(section_row(
            width,
            th,
            focused,
            &format!("[{severity}] {}", finding.message),
            "",
        ));
        if focused {
            rows.extend(section_detail(width, th, &finding.message));
            let mut status = section_detail(width, th, &format!("Severity: {severity}"));
            let color = match finding.severity {
                Severity::Critical => th.error,
                Severity::High => th.warning,
                _ => th.muted,
            };
            for row in &mut status {
                for span in &mut row.spans {
                    span.style.fg = Some(color);
                }
            }
            rows.extend(status);
            for (label, value) in [
                ("Location", &finding.location),
                ("Rule", &finding.rule),
                ("Evidence", &finding.evidence),
                ("Path", &finding.path),
            ] {
                if !value.is_empty() {
                    rows.extend(section_detail(width, th, &format!("{label}: {value}")));
                }
            }
        }
    }
    for (label, value) in [
        ("Report", &scan.report),
        ("Report size", &scan.report_size),
        ("Reported SHA-256", &scan.seal),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let scan = model.security.as_ref();
    SheetChrome {
        header_right: scan
            .filter(|scan| !scan.id.is_empty())
            .map(|scan| vec![span(scan.id.clone(), th.muted)])
            .unwrap_or_default(),
        status_third: scan
            .map(|scan| vec![span(format!("{} findings", scan.findings.len()), th.muted)]),
        status_right: scan
            .and_then(|scan| scan.scanned)
            .filter(|(_, total)| *total > 0)
            .map(|(done, total)| {
                status_meter(
                    th,
                    "scanned",
                    done as f64 / total as f64,
                    &done.to_string(),
                    &total.to_string(),
                )
            }),
        hints: vec![hint(th, "↑↓ finding"), hint(th, "pgup/pgdn read")],
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
        model::SecurityScan,
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
        fixtures::dress_screen(&mut m, "5d");
        m.width = width;
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
    fn selected_finding_expands_its_actual_evidence_rule_and_location() {
        let m = model(160);
        let finding = &m.security.as_ref().unwrap().findings[m.security_index];
        let drawn = text(&m);
        for value in [
            &finding.message,
            &finding.location,
            &finding.rule,
            &finding.evidence,
            &finding.path,
        ] {
            assert!(drawn.contains(value), "{value}");
        }
        assert!(ui::focused_row(&lines(&m)).is_some());
        assert!(drawn.contains(severity_word(finding.severity)));
    }
    #[test]
    fn status_does_not_manufacture_network_validation_or_seal_guarantees() {
        let mut m = model(80);
        m.security = Some(SecurityScan {
            id: "scan-id".into(),
            state: "running".into(),
            ..Default::default()
        });
        let drawn = text(&m);
        for claim in [
            "sealed",
            "not guessed",
            "never left",
            "network false",
            "SHA-256",
        ] {
            assert!(!drawn.contains(claim));
        }
        m.security.as_mut().unwrap().seal = "actual-hash".into();
        assert!(text(&m).contains("Reported SHA-256: actual-hash"));
        assert!(chrome(&m).status_right.is_none());
    }

    #[test]
    fn cancelled_failed_and_incomplete_status_do_not_read_as_success() {
        for state in [
            "cancelled",
            "failed",
            "incomplete",
            "interrupted",
            "unknown",
        ] {
            let mut m = model(80);
            m.security = Some(SecurityScan {
                id: "scan-id".into(),
                state: state.into(),
                ..Default::default()
            });
            let drawn = text(&m);
            assert!(drawn.contains(&format!("Status: {state}")), "{drawn}");
            for claim in ["complete for captured scope", "report sealed", "never left"] {
                assert!(!drawn.contains(claim), "{state}: {drawn}");
            }
        }
    }

    #[test]
    fn missing_scan_is_not_an_authorized_completed_review() {
        let mut m = model(80);
        m.security = None;
        let drawn = text(&m);
        assert!(drawn.contains("No security scan available"));
        assert!(!drawn.contains("Status: completed"));
        assert!(!drawn.contains("report sealed"));
    }
    #[test]
    fn empty_and_unicode_evidence_remain_cell_bounded() {
        let mut m = model(80);
        m.security = None;
        assert!(text(&m).contains("No security scan"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.security.as_mut().unwrap().findings[0].evidence =
                "证据 café 🦀 /long/path ".repeat(20);
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
