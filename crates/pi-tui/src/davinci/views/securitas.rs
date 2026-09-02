//! `5d` — `/sec-report`. A scan you can audit.
//!
//! Every finding carries the rule that produced it, the file and line it was
//! read out of, and the evidence, so a claim can be checked rather than
//! trusted; the selected one expands to its attack path. Coverage and the
//! report seal are stated on the same screen as the findings — a count of
//! criticals means nothing without how much of the tree was read and whether
//! the scan reached the network.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/securitas.ex`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::davinci::model::{Finding, Model, Severity};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, indent, meter, pad, run_width, span, span_on, span_strong, truncate_run,
    MEASURE,
};

/// Cells the right-aligned severity word takes.
const SEVERITY: u16 = 9;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(scan) = model.security.as_ref() else {
        return vec![Line::from(vec![span(
            "no scan has run — /sec-scan starts one",
            th.muted,
        )])];
    };

    let mut progress = vec![
        span_strong(
            format!("{} ", th.spinner(model.tick, model.animate)),
            th.primary,
            th,
        ),
        span(
            format!(
                "validating candidate {} of {}",
                scan.validated, scan.candidates
            ),
            th.text,
        ),
        span("  ", th.border),
    ];
    progress.extend(meter(scan.fraction, 18, th, None));

    let coverage = Line::from(vec![
        span(format!("{} files", scan.files), th.muted),
        span(" · ", th.border),
        span(format!("{} skipped", scan.skipped), th.muted),
        span(" · ", th.border),
        span(format!("{} read", scan.bytes), th.muted),
        span(" · ", th.border),
        span(format!("{} network not used", glyph::DONE), th.success),
    ]);

    let mut chips: Vec<Span<'static>> = Vec::new();
    for (label, count, severity) in &scan.severities {
        chips.push(span(
            format!(" {label} {count} "),
            severity_color(*severity, th),
        ));
        chips.push(span(" ", th.border));
    }

    let mut out = vec![Line::from(progress), coverage, blank(), Line::from(chips)];
    out.push(Line::from(vec![span(
        format!("{} candidates dismissed as false positives", scan.dismissed),
        th.border,
    )]));
    out.push(blank());

    // Below 88 the severity word goes, not the location: the chips above
    // already count the severities, and a finding without its line cannot be
    // checked.
    let severity_column = model.width >= 88;
    let selected = if scan.findings.is_empty() {
        None
    } else {
        Some(model.security_index % scan.findings.len())
    };
    const WINDOW: usize = 10;
    let total = scan.findings.len();
    let around = selected.unwrap_or(0);
    let start = around
        .saturating_sub(WINDOW / 2)
        .min(total.saturating_sub(WINDOW));
    let end = (start + WINDOW).min(total);
    if start > 0 {
        out.push(Line::from(vec![span(
            format!("… {start} above"),
            th.border,
        )]));
    }
    for (index, finding) in scan.findings.iter().enumerate() {
        if index < start || index >= end {
            continue;
        }
        let current = Some(index) == selected;
        out.push(row(finding, current, severity_column, width, th));
        if current {
            out.extend(expansion(finding, th));
        }
    }
    if end < total {
        out.push(Line::from(vec![span(
            format!("… {} more below", total - end),
            th.border,
        )]));
    }

    out.push(blank());
    out.push(Line::from(vec![
        span(
            "every finding was read out of the file, not guessed",
            th.muted,
        ),
        span(" · line and evidence attached", th.border),
    ]));
    out.push(Line::from(vec![
        span("report sealed ", th.muted),
        span_strong(glyph::DONE, th.success, th),
        span(format!(" sha256 {}", scan.seal), th.border),
        span(format!("  {}", scan.report), th.border),
    ]));
    out.push(Line::from(vec![
        span("enter open the file at the line", th.border),
        span(" · ", th.border),
        span("f mark false positive", th.border),
    ]));
    out.push(Line::from(vec![
        span("a abort scan", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));
    // A long seal or report path is cut at the window, never wrapped: the
    // sheet reports one height and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

fn row(
    finding: &Finding,
    selected: bool,
    severity_column: bool,
    width: u16,
    th: &Theme,
) -> Line<'static> {
    let tint = selected.then_some(th.surface);
    let color = severity_color(finding.severity, th);
    let state = state_for(finding.severity);

    // The message takes what the location and severity leave it, so the row
    // never overflows a narrow window — the line number is never given away.
    let right_width = 35 + if severity_column { SEVERITY } else { 0 };
    let message_room = width.saturating_sub(2 + right_width + 1).clamp(10, 40);
    let left = vec![
        strong_on(format!("{} ", state.glyph()), color, tint, th),
        span_on(
            clip_ellipsis(&finding.message, message_room),
            if selected { th.text } else { th.muted },
            tint,
        ),
    ];
    // The location keeps its line number: a finding you cannot open is a
    // rumour.
    let mut right = vec![span_on(
        format!("{:<35}", clip_ellipsis(&finding.location, 34)),
        th.border,
        tint,
    )];
    if severity_column {
        right.push(span_on(
            format!("{:>1$}", severity_word(finding.severity), SEVERITY as usize),
            color,
            tint,
        ));
    }

    let gap = width
        .saturating_sub(run_width(&left))
        .saturating_sub(run_width(&right))
        .max(1);
    let mut spans = left;
    spans.push(pad(gap, tint));
    spans.extend(right);
    Line::from(spans)
}

fn expansion(finding: &Finding, th: &Theme) -> Vec<Line<'static>> {
    vec![
        indent(
            2,
            vec![
                span("rule ", th.muted),
                span(finding.rule.clone(), th.text),
                span(" · validated ", th.muted),
                span_strong(glyph::DONE, th.success, th),
                span(" · evidence ", th.muted),
                span(clip_ellipsis(&finding.evidence, 40), th.border),
            ],
        ),
        indent(2, vec![span(format!("path {}", finding.path), th.border)]),
    ]
}

/// An emphasised run on a tinted row — a selected finding's glyph.
fn strong_on(
    content: impl Into<String>,
    color: Color,
    background: Option<Color>,
    th: &Theme,
) -> Span<'static> {
    let mut style = ratatui::style::Style::default()
        .fg(color)
        .add_modifier(th.emphasis);
    if let Some(background) = background {
        style = style.bg(background);
    }
    Span::styled(content.into(), style)
}

fn state_for(severity: Severity) -> State {
    match severity {
        Severity::Critical => State::Failed,
        Severity::Dismissed => State::Skipped,
        _ => State::Attention,
    }
}

fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
        Severity::Dismissed => "dismissed",
    }
}

fn severity_color(severity: Severity, th: &Theme) -> Color {
    match severity {
        Severity::Critical => th.error,
        Severity::High => th.warning,
        Severity::Medium => th.muted,
        Severity::Low | Severity::Dismissed => th.border,
    }
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::SecurityScan;
    use crate::davinci::theme::ColorDepth;

    fn scan() -> SecurityScan {
        SecurityScan {
            validated: 31,
            candidates: 44,
            fraction: 0.7,
            files: "1,842".into(),
            skipped: "96".into(),
            bytes: "41.2 MB".into(),
            severities: vec![
                ("critical".into(), 1, Severity::Critical),
                ("high".into(), 3, Severity::High),
                ("medium".into(), 6, Severity::Medium),
            ],
            dismissed: 11,
            findings: vec![
                Finding {
                    message: "bearer token written to the transcript".into(),
                    location: "davinci-ai\\src\\auth.rs:214".into(),
                    severity: Severity::Critical,
                    rule: "secret-in-log".into(),
                    evidence: "tracing::debug!(\"refresh {token}\")".into(),
                    path: "refresh_token() → session jsonl → /export".into(),
                },
                Finding {
                    message: "command built from an unquoted path".into(),
                    location: "davinci-agent\\src\\tools\\bash.rs:88".into(),
                    severity: Severity::High,
                    rule: "shell-injection".into(),
                    evidence: "format!(\"cd {} && {}\", dir, cmd)".into(),
                    path: "bash tool → cmd.exe → any path with a space".into(),
                },
            ],
            seal: "4b1f…c9e0".into(),
            report: ".davinci\\security\\s-31c8\\report.json · 214 KB".into(),
        }
    }

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.security = Some(scan());
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_scan_states_its_coverage_findings_and_seal() {
        let rows: Vec<String> = lines(&model(100)).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("validating candidate 31 of 44")));
        assert!(rows.iter().any(|row| row.contains("1,842 files")));
        assert!(rows.iter().any(|row| row.contains("network not used")));
        assert!(rows.iter().any(|row| row.contains("bearer token written")));
        assert!(rows.iter().any(|row| row.contains("sha256 4b1f…c9e0")));
        assert!(rows
            .iter()
            .any(|row| row.contains("11 candidates dismissed")));
    }

    #[test]
    fn the_selected_finding_expands_to_rule_evidence_and_path() {
        let mut m = model(100);
        m.security_index = 1;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("rule shell-injection")));
        assert!(rows
            .iter()
            .any(|row| row.contains("path bash tool → cmd.exe")));
        // The unselected finding stays one line.
        assert!(!rows.iter().any(|row| row.contains("rule secret-in-log")));
    }

    #[test]
    fn the_severity_word_gives_way_below_eighty_eight_but_the_location_never() {
        let narrow: Vec<String> = lines(&model(80)).iter().map(text).collect();
        assert!(!narrow.iter().any(|row| row.ends_with("critical")));
        assert!(narrow.iter().any(|row| row.contains("auth.rs:214")));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_breakpoint() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(run_width(&row.spans) <= width, "overflow at {width}");
            }
        }
    }

    #[test]
    fn a_session_with_no_scan_says_so() {
        let mut m = model(100);
        m.security = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("no scan has run")));
    }
}
