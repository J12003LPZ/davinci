//! `5d` — `/sec-report`. A scan you can audit.
//!
//! Every finding carries the rule that produced it, the file and line it was
//! read out of, and the evidence, so a claim can be checked rather than
//! trusted; the selected one expands to its attack path. Coverage and the
//! report seal are stated on the same screen as the findings — a count of
//! criticals means nothing without how much of the tree was read and whether
//! the scan reached the network.
//!
//! Mirrors artboard `5d` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::Color;
use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, status_meter, Composer, SheetChrome};
use crate::davinci::model::{Finding, Model, Severity};
use crate::davinci::theme::{glyph, State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, footnote, indent, meter, pad, run_width, selection_bar, span, span_on,
    span_strong, truncate_run, wrap,
};

/// Cells the right-aligned severity word takes.
const SEVERITY: u16 = 9;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(scan) = model.security.as_ref() else {
        return vec![Line::from(vec![span(
            "no scan has run — /sec-scan starts one",
            th.muted,
        )])];
    };

    // No scan has been started in this project: say how one starts rather
    // than drawing a meter over nothing.
    if scan.id.is_empty() && scan.candidates == 0 && scan.findings.is_empty() {
        let mut out = vec![Line::from(vec![span(
            "no security scan in this project yet",
            th.text,
        )])];
        out.push(blank());
        for row in wrap(
            "ask the agent to run a security scan: it starts one with sec_scan_start, \
             validates each candidate against the file it names, and this sheet follows \
             along; /sec-report writes the report once the scan is complete",
            width,
        ) {
            out.push(Line::from(vec![span(row, th.muted)]));
        }
        out.push(blank());
        out.push(Line::from(vec![span(
            "the scan never leaves this machine · allow_network false",
            th.border,
        )]));
        return out;
    }

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
    progress.extend(meter(scan.fraction, 24, th, None));

    let coverage = Line::from(vec![
        span(format!("{} files", scan.files), th.muted),
        span(" · ", th.border),
        span(format!("{} skipped", scan.skipped), th.muted),
        span(" · ", th.border),
        span(format!("{} read", scan.bytes), th.muted),
    ]);

    let mut tally: Vec<Span<'static>> = Vec::new();
    for (index, (label, count, severity)) in scan.severities.iter().enumerate() {
        if index > 0 {
            tally.push(span("   ", th.border));
        }
        tally.push(span(format!("{label} "), severity_color(*severity, th)));
        tally.push(span(count.to_string(), th.text));
    }

    let mut out = vec![Line::from(progress), coverage, blank(), Line::from(tally)];
    out.push(Line::from(vec![span(
        format!("{} candidates dismissed as false positives", scan.dismissed),
        th.border,
    )]));
    out.push(blank());

    // Below 88 the severity word goes, not the location: the tally above
    // already counts the severities, and a finding without its line cannot be
    // checked.
    let severity_column = model.width >= 88;
    let selected = if scan.findings.is_empty() {
        None
    } else {
        Some(model.security_index % scan.findings.len())
    };
    for (index, finding) in scan.findings.iter().enumerate() {
        let current = Some(index) == selected;
        out.push(row(finding, current, severity_column, width, th));
        if current {
            out.extend(expansion(finding, width, th));
        }
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
        span("the scan never left this machine", th.muted),
        span(" · allow_network false", th.border),
    ]));
    let mut sealed = vec![
        span("report sealed ", th.muted),
        span_strong(glyph::DONE, th.success, th),
        span(format!(" sha256 {}", scan.seal), th.text),
    ];
    if !scan.report_size.is_empty() {
        sealed.push(span(format!(" · {}", scan.report_size), th.border));
    }
    out.extend(footnote(
        width,
        sealed,
        vec![span(scan.report.clone(), th.secondary)],
        th,
    ));
    // A long seal or report path is cut at the window, never wrapped: the
    // sheet reports one height and keeps it.
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the scan in the header, the critical
/// count in the status bar with the files scanned as its meter.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let scan = model.security.as_ref();
    let critical = scan
        .and_then(|s| {
            s.severities
                .iter()
                .find(|(_, _, severity)| *severity == Severity::Critical)
        })
        .map(|(_, count, _)| *count);
    SheetChrome {
        header_right: facts(
            th,
            vec![
                scan.filter(|s| !s.id.is_empty())
                    .map(|s| vec![span("scan ", th.muted), span(s.id.clone(), th.text)])
                    .unwrap_or_default(),
                scan.filter(|s| !s.state.is_empty())
                    .map(|s| vec![span(s.state.clone(), th.muted)])
                    .unwrap_or_default(),
                scan.map(|_| {
                    vec![
                        span("network ", th.muted),
                        span(format!("{} not used", glyph::DONE), th.success),
                    ]
                })
                .unwrap_or_default(),
            ],
        ),
        status_third: critical.map(|n| {
            vec![span(
                format!("{n} critical"),
                if n > 0 { th.error } else { th.muted },
            )]
        }),
        status_right: scan
            .and_then(|s| s.scanned)
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
        hints: vec![
            hint(th, "enter open the file at the line"),
            hint_dim(th, "f mark false positive"),
            hint_dim(th, "p show attack path"),
            hint_dim(th, "a abort scan"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("/sec-report --severity high"),
        echo: None,
    }
}

fn row(
    finding: &Finding,
    selected: bool,
    severity_column: bool,
    width: u16,
    th: &Theme,
) -> Line<'static> {
    let tint = selected.then_some(th.surface);
    let dismissed = finding.severity == Severity::Dismissed;
    let ink = if dismissed { th.dim() } else { *th };
    let color = severity_color(finding.severity, &ink);
    let state = state_for(finding.severity);

    // The message takes what the location and severity leave it, so the row
    // never overflows a narrow window — the line number is never given away.
    let right_width = 35 + if severity_column { SEVERITY } else { 0 };
    let message_room = width.saturating_sub(LEAD + right_width + 1).clamp(10, 60);
    let left = vec![
        selection_bar(selected, th),
        strong_on(format!("{} ", state.glyph()), color, tint, th),
        span_on(
            clip_ellipsis(&finding.message, message_room),
            if selected {
                th.text
            } else if dismissed {
                ink.muted
            } else {
                th.muted
            },
            tint,
        ),
    ];
    // The location keeps its line number: a finding you cannot open is a
    // rumour.
    let mut right = vec![span_on(
        format!("{:<35}", clip_ellipsis(&finding.location, 34)),
        ink.border,
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

fn expansion(finding: &Finding, width: u16, th: &Theme) -> Vec<Line<'static>> {
    vec![
        indent(
            LEAD,
            vec![
                span("rule ", th.muted),
                span(finding.rule.clone(), th.text),
                span(" · validated ", th.muted),
                span_strong(glyph::DONE, th.success, th),
                span(" · evidence ", th.muted),
                span(
                    clip_ellipsis(&finding.evidence, width.saturating_sub(LEAD + 40)),
                    th.border,
                ),
            ],
        ),
        indent(
            LEAD,
            vec![
                span("path ", th.muted),
                span(
                    clip_ellipsis(&finding.path, width.saturating_sub(LEAD + 5)),
                    th.border,
                ),
            ],
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
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
                Finding {
                    message: "hard-coded test key in a fixture".into(),
                    location: "tests\\fixtures\\auth.json:3".into(),
                    severity: Severity::Dismissed,
                    rule: "secret-literal".into(),
                    evidence: "\"api_key\": \"sk-test-0000\"".into(),
                    path: "not a real credential".into(),
                },
            ],
            seal: "4b1f…c9e0".into(),
            report: ".pi\\security\\s-31c8\\report.json".into(),
            report_size: "214 KB".into(),
            id: "s-31c8".into(),
            state: "draft".into(),
            scanned: Some((1842, 2140)),
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
        assert!(rows
            .iter()
            .any(|row| row.starts_with("critical 1") && row.contains("high 3")));
        assert!(rows
            .iter()
            .any(|row| row.contains("bearer token written to the transcript")
                && row.contains("auth.rs:214")
                && row.trim_end().ends_with("critical")));
        assert!(rows.iter().any(|row| row.contains("rule secret-in-log")));
        assert!(rows
            .iter()
            .any(|row| row.contains("the scan never left this machine")));
        assert!(rows.iter().any(
            |row| row.contains("report sealed ✓ sha256 4b1f…c9e0 · 214 KB")
                && row.contains(".pi\\security\\s-31c8\\report.json")
        ));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn the_selected_finding_is_marked_and_expanded_and_a_dismissed_one_dims() {
        let m = model(100);
        let rows = lines(&m);
        let selected = rows
            .iter()
            .find(|row| text(row).contains("bearer token"))
            .unwrap();
        assert!(text(selected).starts_with("▌  ×"), "{}", text(selected));
        let dismissed = rows
            .iter()
            .find(|row| text(row).contains("hard-coded test key"))
            .unwrap();
        assert!(text(dismissed).starts_with("   ◌"), "{}", text(dismissed));
        assert!(dismissed
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.dim().muted)));
        // Only the selected finding carries its rule and path.
        let drawn: Vec<String> = rows.iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("rule shell-injection")));
    }

    #[test]
    fn below_eighty_eight_columns_the_severity_word_goes_not_the_line() {
        let rows: Vec<String> = lines(&model(80)).iter().map(text).collect();
        let row = rows
            .iter()
            .find(|row| row.contains("bearer token"))
            .unwrap();
        assert!(row.contains("auth.rs:214"), "{row}");
        assert!(!row.trim_end().ends_with("critical"), "{row}");
    }

    #[test]
    fn no_scan_says_how_to_start_one() {
        let mut m = model(100);
        m.security = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("/sec-scan starts one")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "5d");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "scan s-31c8 │ draft │ network ✓ not used");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "1 critical");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(right.starts_with("scanned "), "{right}");
        assert!(right.ends_with(" 1842/2140"), "{right}");
        assert_eq!(c.composer, Composer::Prompt("/sec-report --severity high"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("enter open the file at the line │ f mark false positive"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
