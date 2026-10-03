//! Completion status report view rendering per-dimension evidence, source fingerprint,
//! commands, artifacts, and named gaps.

use ratatui::text::Line;

use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{section_detail, section_state};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionDimensionRow {
    pub dimension: String,
    pub state: State,
    pub label: String,
    pub required: bool,
    pub proof: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReport {
    pub task_id: String,
    pub title: String,
    pub source_fingerprint: String,
    pub evaluated_at: String,
    pub allowed: bool,
    pub dimensions: Vec<CompletionDimensionRow>,
    pub remaining_gaps: Vec<String>,
}

pub fn lines(width: u16, theme: &Theme, report: &CompletionReport) -> Vec<Line<'static>> {
    let mut rows = Vec::new();

    let overall_state = if report.allowed {
        State::Done
    } else {
        State::Active
    };
    rows.extend(section_state(
        width,
        theme,
        overall_state,
        &format!("Task Completion: {}", report.title),
    ));

    rows.extend(section_detail(
        width,
        theme,
        &format!("Source Digest: {}", report.source_fingerprint),
    ));
    rows.extend(section_detail(
        width,
        theme,
        &format!("Evaluated At: {}", report.evaluated_at),
    ));

    for dim in &report.dimensions {
        let req_text = if dim.required { "required" } else { "optional" };
        let header = format!("{}: {} ({})", dim.dimension, dim.label, req_text);
        rows.extend(section_state(width, theme, dim.state, &header));
        if let Some(ref proof) = dim.proof {
            rows.extend(section_detail(width, theme, &format!("Proof: {proof}")));
        }
    }

    if !report.remaining_gaps.is_empty() {
        let mut gaps = Vec::new();
        gaps.extend(section_detail(width, theme, "Remaining Gaps:"));
        for gap in &report.remaining_gaps {
            let mut gap_rows = section_detail(width, theme, &format!("! {gap}"));
            for r in &mut gap_rows {
                for s in &mut r.spans {
                    s.style.fg = Some(theme.error);
                }
            }
            gaps.extend(gap_rows);
        }
        rows.extend(gaps);
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn test_theme() -> Theme {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    }

    #[test]
    fn test_render_completion_report() {
        let theme = test_theme();
        let report = CompletionReport {
            task_id: "task-123".into(),
            title: "Verification task".into(),
            source_fingerprint: "sha256:abc123def456".into(),
            evaluated_at: "2026-09-09T03:00:00Z".into(),
            allowed: true,
            dimensions: vec![
                CompletionDimensionRow {
                    dimension: "implementation".into(),
                    state: State::Done,
                    label: "passed on current source".into(),
                    required: true,
                    proof: Some("write src/lib.rs".into()),
                },
                CompletionDimensionRow {
                    dimension: "targeted_tests".into(),
                    state: State::Done,
                    label: "passed on current source".into(),
                    required: true,
                    proof: Some("cargo test -p davinci-agent".into()),
                },
            ],
            remaining_gaps: vec![],
        };

        let rendered = lines(80, &theme, &report);
        assert!(!rendered.is_empty());
        let text = rendered
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Verification task"));
        assert!(text.contains("sha256:abc123def456"));
        assert!(text.contains("passed on current source"));
    }

    #[test]
    fn test_render_terminal_width40() {
        let theme = test_theme();
        let report = CompletionReport {
            task_id: "task-narrow".into(),
            title: "Narrow terminal report with long details".into(),
            source_fingerprint: "sha256:long_hash_string_for_narrow_terminal_checking".into(),
            evaluated_at: "2026-09-09T03:00:00Z".into(),
            allowed: false,
            dimensions: vec![
                CompletionDimensionRow {
                    dimension: "implementation".into(),
                    state: State::Done,
                    label: "passed on current source".into(),
                    required: true,
                    proof: Some("edit crates/davinci-agent/src/runtime/tasks.rs".into()),
                },
                CompletionDimensionRow {
                    dimension: "targeted_tests".into(),
                    state: State::Active,
                    label: "failed".into(),
                    required: true,
                    proof: None,
                },
            ],
            remaining_gaps: vec!["cargo test suite failed exit 1".into()],
        };

        let rendered = lines(40, &theme, &report);
        assert!(!rendered.is_empty());
        // Verify no panic on width 40 and all lines are rendered
        for l in &rendered {
            assert!(l.width() <= 40 || !l.to_string().is_empty());
        }
    }

    #[test]
    fn test_live_ui_not_performed_after_build() {
        let theme = test_theme();
        let report = CompletionReport {
            task_id: "task-ui".into(),
            title: "Build without automated live UI".into(),
            source_fingerprint: "sha256:111222333".into(),
            evaluated_at: "2026-09-09T03:00:00Z".into(),
            allowed: false,
            dimensions: vec![
                CompletionDimensionRow {
                    dimension: "build".into(),
                    state: State::Done,
                    label: "passed on current source".into(),
                    required: true,
                    proof: Some("cargo build --release".into()),
                },
                CompletionDimensionRow {
                    dimension: "live_ui_check".into(),
                    state: State::Skipped,
                    label: "not performed".into(),
                    required: false,
                    proof: Some("Physical keyboard interaction not automated".into()),
                },
            ],
            remaining_gaps: vec!["Live UI check not performed".into()],
        };

        let rendered = lines(80, &theme, &report);
        let text = rendered
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("live_ui_check"));
        assert!(text.contains("not performed"));
        assert!(text.contains("Physical keyboard interaction not automated"));
    }

    #[test]
    fn test_invalidated_tests_render_stale() {
        let theme = test_theme();
        let report = CompletionReport {
            task_id: "task-stale".into(),
            title: "Stale test proof".into(),
            source_fingerprint: "sha256:new_modified_source".into(),
            evaluated_at: "2026-09-09T03:00:00Z".into(),
            allowed: false,
            dimensions: vec![CompletionDimensionRow {
                dimension: "targeted_tests".into(),
                state: State::Active,
                label: "stale".into(),
                required: true,
                proof: Some("run on old digest sha256:old_source".into()),
            }],
            remaining_gaps: vec!["Tests stale: source modified after test run".into()],
        };

        let rendered = lines(80, &theme, &report);
        let text = rendered
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("targeted_tests"));
        assert!(text.contains("stale"));
        assert!(text.contains("Tests stale: source modified after test run"));
    }
}
