//! Whole-task resource budget and progress watchdog view.
//! Matching spec: docs/superpowers/plans/2026-09-07-davinci-feature-specs/09-whole-task-budgets-loop-detection.md

use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_heading};
use ratatui::text::Line;

/// Format duration in mm:ss.
pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins:02}:{secs:02}")
}

/// Format token count with thousands separator or 'k' notation.
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// Generate lines for the whole-task budget view.
pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

    let Some(budget) = &model.task_budget else {
        return section_detail(
            width,
            th,
            "Whole-task resource budget is not configured for this run.",
        );
    };

    let mut rows = section_heading(width, th, "Budget");

    // Tokens
    let tokens_line = format!(
        "Tokens        {} / {}",
        format_tokens(budget.tokens_charged),
        format_tokens(budget.token_ceiling)
    );
    rows.extend(section_detail(width, th, &tokens_line));

    // Elapsed
    let elapsed_line = format!(
        "Elapsed       {} / {}",
        format_duration_ms(budget.elapsed_ms),
        format_duration_ms(budget.deadline_ms)
    );
    rows.extend(section_detail(width, th, &elapsed_line));

    // Workers
    let workers_line = format!(
        "Workers       {} / {} concurrent",
        budget.active_workers, budget.max_concurrency
    );
    rows.extend(section_detail(width, th, &workers_line));

    // Retries
    let retries_line = format!(
        "Retries       {} / {}",
        budget.retries_used, budget.retry_ceiling
    );
    rows.extend(section_detail(width, th, &retries_line));

    // Cost
    let cost_line = format!("Cost          {}", budget.cost_label);
    rows.extend(section_detail(width, th, &cost_line));

    // Protected Reserves
    let reserves_line = format!(
        "Reserves      {} verify · {} handoff",
        format_tokens(budget.verification_reserve),
        format_tokens(budget.handoff_reserve)
    );
    rows.extend(section_detail(width, th, &reserves_line));

    // Progress watchdog section if a signal is active
    if let Some(signal) = &budget.watchdog_signal {
        rows.extend(section_heading(width, th, "Progress watchdog"));
        rows.extend(section_detail(width, th, &format!("! {signal}")));
        rows.extend(section_detail(
            width,
            th,
            "› 1. Continue with remaining budget",
        ));
        rows.extend(section_detail(
            width,
            th,
            "  2. Return to Plan Mode with evidence",
        ));
        rows.extend(section_detail(
            width,
            th,
            "  3. Stop and preserve checkpoint",
        ));
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::{Model, TaskBudgetView};
    use crate::davinci::theme::{ColorDepth, Theme};

    fn test_model() -> Model {
        Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false)
    }

    #[test]
    fn test_budget_view_rendering_spec_layout() {
        let mut model = test_model();
        model.task_budget = Some(TaskBudgetView {
            tokens_charged: 81_000,
            token_ceiling: 120_000,
            reserved_tokens: 0,
            elapsed_ms: 7 * 60 * 1000 + 18 * 1000,
            deadline_ms: 15 * 60 * 1000,
            active_workers: 3,
            max_concurrency: 4,
            retries_used: 2,
            retry_ceiling: 5,
            cost_label: "unknown (provider pricing unavailable)".into(),
            verification_reserve: 18_000,
            handoff_reserve: 6_000,
            watchdog_signal: Some(
                "Same test failure observed across 3 attempts with no new diagnosis.".into(),
            ),
        });

        let rendered = lines(&model);
        let text: Vec<String> = rendered
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect();

        let joined = text.join("\n");
        assert!(joined.contains("Tokens") && joined.contains("81k / 120k"));
        assert!(joined.contains("Elapsed") && joined.contains("07:18 / 15:00"));
        assert!(joined.contains("Workers") && joined.contains("3 / 4 concurrent"));
        assert!(joined.contains("Retries") && joined.contains("2 / 5"));
        assert!(
            joined.contains("Cost") && joined.contains("unknown (provider pricing unavailable)")
        );
        assert!(joined.contains("Reserves") && joined.contains("18k verify · 6k handoff"));
        assert!(joined.contains("Progress watchdog"));
        assert!(joined
            .contains("! Same test failure observed across 3 attempts with no new diagnosis."));
        assert!(joined.contains("1. Continue with remaining budget"));
        assert!(joined.contains("2. Return to Plan Mode with evidence"));
        assert!(joined.contains("3. Stop and preserve checkpoint"));
    }

    #[test]
    fn test_budget_view_empty_state() {
        let model = test_model();
        let rendered = lines(&model);
        let joined = rendered
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("not configured"));
    }
}
