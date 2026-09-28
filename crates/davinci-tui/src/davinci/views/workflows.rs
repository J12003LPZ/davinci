//! `/workflows` — workflow runs, drilled into like claude code's progress
//! view: the runs, one run's phases (agent counts, tokens, elapsed), one
//! phase's agents, and one agent's calls and result.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{
    Model, WorkflowAgentRow, WorkflowLevel, WorkflowPhaseRow, WorkflowRow, WorkflowsSheet,
};
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_row, section_state, span};
use ratatui::text::Line;

/// Rows of an agent's result shown before `enter` expands it.
const DETAIL_ROWS: usize = 12;

fn state(status: &str) -> State {
    match status {
        "completed" => State::Done,
        "running" => State::Active,
        "paused" | "pending" => State::Queued,
        "cancelled" | "skipped" => State::Skipped,
        _ => State::Failed,
    }
}

fn tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}m", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn seconds(total: u64) -> String {
    if total >= 60 {
        format!("{}m {}s", total / 60, total % 60)
    } else {
        format!("{total}s")
    }
}

fn phase_totals(phase: &WorkflowPhaseRow) -> (usize, u64, u64) {
    let done = phase
        .agents
        .iter()
        .filter(|agent| agent.status == "completed")
        .count();
    let tokens = phase.agents.iter().map(|agent| agent.tokens).sum();
    let elapsed = phase
        .agents
        .iter()
        .map(|agent| agent.elapsed_secs)
        .max()
        .unwrap_or(0);
    (done, tokens, elapsed)
}

fn run_tokens(run: &WorkflowRow) -> u64 {
    run.phase_rows
        .iter()
        .flat_map(|phase| &phase.agents)
        .map(|agent| agent.tokens)
        .sum()
}

fn error_rows(model: &Model, text: &str) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut rows = section_detail(model.width, th, text);
    for row in &mut rows {
        for span in &mut row.spans {
            span.style.fg = Some(th.error);
        }
    }
    rows
}

fn runs(model: &Model, sheet: &WorkflowsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let mut rows = Vec::new();
    for (index, run) in sheet.workflows.iter().enumerate() {
        let selected = index == sheet.selected_index;
        rows.push(section_row(
            width,
            th,
            selected,
            &format!("{} · {}", run.name, run.status),
            &run.elapsed,
        ));
        let agents: usize = run.phase_rows.iter().map(|phase| phase.agents.len()).sum();
        let done_phases = run
            .phases
            .iter()
            .filter(|(_, status)| status == "completed")
            .count();
        let mut facts = vec![format!(
            "{done_phases}/{} phases · {agents} agents",
            run.phases.len()
        )];
        let spent = run_tokens(run);
        if spent > 0 {
            facts.push(format!("{} tokens", tokens(spent)));
        }
        facts.push(run.id.clone());
        rows.extend(section_detail(width, th, &facts.join(" · ")));
        if let Some(error) = &run.error {
            rows.extend(error_rows(model, error));
        }
    }
    rows
}

fn phases(model: &Model, sheet: &WorkflowsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(run) = sheet.run() else {
        return Vec::new();
    };
    let mut rows = section_state(
        width,
        th,
        state(&run.status),
        &format!("{} · {} · {}", run.name, run.status, run.elapsed),
    );
    if let Some(error) = &run.error {
        rows.extend(error_rows(model, error));
    }
    for (index, phase) in run.phase_rows.iter().enumerate() {
        let (done, spent, elapsed) = phase_totals(phase);
        let mut right = Vec::new();
        if spent > 0 {
            right.push(format!("{} tokens", tokens(spent)));
        }
        if elapsed > 0 {
            right.push(seconds(elapsed));
        }
        rows.push(section_row(
            width,
            th,
            index == sheet.phase_index,
            &format!(
                "{} {} · {} · {done}/{} agents",
                state(&phase.status).glyph(),
                phase.id,
                phase.status,
                phase.agents.len()
            ),
            &right.join(" · "),
        ));
    }
    rows
}

/// `12.3k tokens · 1m 5s`: what fits the right-hand column of a row.
fn agent_cost(agent: &WorkflowAgentRow) -> String {
    let mut parts = Vec::new();
    if agent.tokens > 0 {
        parts.push(format!("{} tokens", tokens(agent.tokens)));
    }
    if agent.elapsed_secs > 0 {
        parts.push(seconds(agent.elapsed_secs));
    }
    parts.join(" · ")
}

fn tool_uses(count: u64) -> String {
    if count == 1 {
        "1 tool use".into()
    } else {
        format!("{count} tool uses")
    }
}

fn agent_stats(agent: &WorkflowAgentRow) -> String {
    let mut parts = Vec::new();
    if agent.tool_uses > 0 {
        parts.push(if agent.tool_uses == 1 {
            "1 tool use".to_string()
        } else {
            format!("{} tool uses", agent.tool_uses)
        });
    }
    if agent.tokens > 0 {
        parts.push(format!("{} tokens", tokens(agent.tokens)));
    }
    if agent.elapsed_secs > 0 {
        parts.push(seconds(agent.elapsed_secs));
    }
    parts.join(" · ")
}

fn agents(model: &Model, sheet: &WorkflowsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let (Some(run), Some(phase)) = (sheet.run(), sheet.phase()) else {
        return Vec::new();
    };
    let filter = sheet
        .filter
        .as_deref()
        .map(|status| format!(" · showing {status}"))
        .unwrap_or_default();
    let mut rows = section_state(
        width,
        th,
        state(&phase.status),
        &format!("{} › {} · {}{filter}", run.name, phase.id, phase.status),
    );
    let visible = sheet.visible_agents();
    if visible.is_empty() {
        rows.extend(section_detail(width, th, "No agents match this filter."));
    }
    for (index, agent) in visible.iter().enumerate() {
        let selected = index == sheet.agent_index;
        rows.push(section_row(
            width,
            th,
            selected,
            &format!(
                "{} {} · {} · {}",
                state(&agent.status).glyph(),
                agent.label,
                agent.status,
                tool_uses(agent.tool_uses)
            ),
            &agent_cost(agent),
        ));
        if selected {
            if let Some(last) = agent.recent.last() {
                rows.extend(section_detail(width, th, &format!("⎿  {last}")));
            }
        }
    }
    rows
}

fn agent_detail(model: &Model, sheet: &WorkflowsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let (Some(run), Some(phase), Some(agent)) = (sheet.run(), sheet.phase(), sheet.agent())
    else {
        return Vec::new();
    };
    let mut rows = section_state(
        width,
        th,
        state(&agent.status),
        &format!("{} › {} › {} · {}", run.name, phase.id, agent.label, agent.status),
    );
    let stats = agent_stats(agent);
    if !stats.is_empty() {
        rows.extend(section_detail(width, th, &stats));
    }
    rows.extend(section_detail(width, th, &format!("agent {}", agent.agent_id)));
    if !agent.recent.is_empty() {
        rows.extend(section_detail(width, th, "Recent calls"));
        let calls: Vec<&String> = if sheet.expanded {
            agent.recent.iter().collect()
        } else {
            agent.recent.iter().rev().take(4).rev().collect()
        };
        for call in calls {
            rows.extend(section_detail(width, th, &format!("  {call}")));
        }
    }
    if let Some(detail) = agent.detail.as_deref().filter(|text| !text.trim().is_empty()) {
        let heading = if agent.status == "failed" { "Error" } else { "Result" };
        rows.extend(section_detail(width, th, heading));
        let lines: Vec<&str> = detail.lines().collect();
        let shown = if sheet.expanded {
            lines.len()
        } else {
            lines.len().min(DETAIL_ROWS)
        };
        for line in &lines[..shown] {
            if agent.status == "failed" {
                rows.extend(error_rows(model, &format!("  {line}")));
            } else {
                rows.extend(section_detail(width, th, &format!("  {line}")));
            }
        }
        if shown < lines.len() {
            rows.extend(section_detail(
                width,
                th,
                &format!("  … {} more lines (enter to expand)", lines.len() - shown),
            ));
        }
    }
    rows
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .workflows
        .as_ref()
        .filter(|sheet| !sheet.workflows.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No workflow runs in this session. Use /workflow <name|goal> to start one.",
        );
    };
    let mut rows = sheet
        .notice
        .as_deref()
        .map(|notice| section_detail(width, th, notice))
        .unwrap_or_default();
    rows.extend(match sheet.level {
        WorkflowLevel::Runs => runs(model, sheet),
        WorkflowLevel::Phases => phases(model, sheet),
        WorkflowLevel::Agents => agents(model, sheet),
        WorkflowLevel::Agent => agent_detail(model, sheet),
    });
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let sheet = model.workflows.as_ref();
    let total = sheet.map_or(0, |sheet| sheet.workflows.len());
    let running = sheet.map_or(0, |sheet| {
        sheet
            .workflows
            .iter()
            .filter(|workflow| workflow.status == "running")
            .count()
    });
    let level = sheet.map_or(WorkflowLevel::Runs, |sheet| sheet.level);
    let mut hints = vec![hint(th, "↑↓ select")];
    match level {
        WorkflowLevel::Runs => {
            hints.push(hint(th, "enter open"));
            hints.push(hint(th, "p pause"));
            hints.push(hint(th, "x stop"));
        }
        WorkflowLevel::Phases => {
            hints.push(hint(th, "enter agents"));
            hints.push(hint(th, "p pause"));
            hints.push(hint(th, "x stop run"));
        }
        WorkflowLevel::Agents => {
            hints.push(hint(th, "enter detail"));
            hints.push(hint(th, "f filter"));
            hints.push(hint(th, "x stop agent"));
        }
        WorkflowLevel::Agent => {
            hints.push(hint(th, "enter expand"));
            hints.push(hint(th, "x stop agent"));
        }
    }
    SheetChrome {
        header_right: vec![span(format!("{total} workflows"), th.muted)],
        status_third: Some(vec![span(format!("{running} running"), th.muted)]),
        hints,
        escape: Some(if level == WorkflowLevel::Runs {
            "esc close"
        } else {
            "esc back"
        }),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{WorkflowRow, WorkflowsSheet},
        theme::{ColorDepth, Theme},
        ui,
    };

    fn agent(label: &str, status: &str, uses: u64) -> WorkflowAgentRow {
        WorkflowAgentRow {
            agent_id: format!("id-{label}"),
            label: label.into(),
            status: status.into(),
            tool_uses: uses,
            tokens: 12_300,
            elapsed_secs: 65,
            recent: vec!["Read(a.rs)".into(), "Search(\"auth\")".into()],
            detail: Some(if status == "failed" {
                "provider down".into()
            } else {
                (1..=20).map(|n| format!("finding {n}")).collect::<Vec<_>>().join("\n")
            }),
        }
    }

    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.workflows = Some(WorkflowsSheet {
            workflows: vec![WorkflowRow {
                id: "workflow-full-identifier-12345".into(),
                name: "Review changes".into(),
                status: "paused".into(),
                phases: vec![
                    ("analyze".into(), "completed".into()),
                    ("review".into(), "paused".into()),
                ],
                started_ms: 0,
                elapsed: "3m 12s".into(),
                error: Some("Worker unavailable".into()),
                phase_rows: vec![
                    WorkflowPhaseRow {
                        id: "analyze".into(),
                        status: "completed".into(),
                        agents: vec![agent("scan", "completed", 7)],
                    },
                    WorkflowPhaseRow {
                        id: "review".into(),
                        status: "paused".into(),
                        agents: vec![
                            agent("security", "running", 3),
                            agent("perf", "failed", 1),
                        ],
                    },
                ],
            }],
            ..WorkflowsSheet::default()
        });
        m
    }

    fn drawn(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_run_list_keeps_identifiers_totals_and_errors() {
        let m = model(100);
        let text = drawn(&m);
        for value in [
            "workflow-full-identifier-12345",
            "Review changes · paused",
            "1/2 phases · 3 agents",
            "36.9k tokens",
            "3m 12s",
            "Worker unavailable",
        ] {
            assert!(text.contains(value), "{value} missing:\n{text}");
        }
        assert!(ui::focused_row(&lines(&m)).is_some());
    }

    #[test]
    fn drilling_shows_phases_agents_and_an_agent_detail() {
        let mut m = model(100);
        let sheet = m.workflows.as_mut().unwrap();
        sheet.drill_in();
        let text = drawn(&m);
        assert!(text.contains("analyze · completed · 1/1 agents"), "{text}");
        assert!(text.contains("12.3k tokens · 1m 5s"), "{text}");
        assert!(text.contains("review · paused · 0/2 agents"), "{text}");

        let sheet = m.workflows.as_mut().unwrap();
        sheet.move_selection(1);
        sheet.drill_in();
        let text = drawn(&m);
        assert!(text.contains("Review changes › review"), "{text}");
        assert!(text.contains("security · running · 3 tool uses"), "{text}");
        assert!(text.contains("12.3k tokens · 1m 5s"), "{text}");
        assert!(text.contains("⎿ Search(\"auth\")"), "{text}");

        let sheet = m.workflows.as_mut().unwrap();
        sheet.cycle_filter();
        assert_eq!(sheet.visible_agents().len(), 1);
        sheet.cycle_filter();
        sheet.cycle_filter();
        assert_eq!(sheet.visible_agents()[0].label, "perf");
        sheet.filter = None;
        sheet.drill_in();
        let text = drawn(&m);
        assert!(text.contains("Result"), "{text}");
        assert!(text.contains("more lines (enter to expand)"), "{text}");
        m.workflows.as_mut().unwrap().drill_in();
        let text = drawn(&m);
        assert!(text.contains("finding 20"), "{text}");
        assert!(!text.contains("more lines"), "{text}");

        let sheet = m.workflows.as_mut().unwrap();
        assert!(sheet.back_out());
        assert!(sheet.back_out());
        assert!(sheet.back_out());
        assert!(!sheet.back_out(), "the run list closes the sheet");
    }

    #[test]
    fn a_refresh_keeps_the_view_on_the_same_agent() {
        let mut m = model(100);
        let sheet = m.workflows.as_mut().unwrap();
        sheet.drill_in();
        sheet.move_selection(1);
        sheet.drill_in();
        sheet.move_selection(1);
        assert_eq!(sheet.agent().unwrap().label, "perf");
        let mut rows = sheet.workflows.clone();
        rows[0].phase_rows[1].agents.reverse();
        sheet.refresh(rows);
        assert_eq!(sheet.agent().unwrap().label, "perf");
        sheet.refresh(Vec::new());
        assert_eq!(sheet.level, WorkflowLevel::Runs);
    }

    #[test]
    fn empty_and_narrow_workflows_are_bounded() {
        let mut m = model(80);
        m.workflows = None;
        assert!(lines(&m)[0].to_string().contains("No workflow runs"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            for _ in 0..4 {
                for row in lines(&m) {
                    assert!(ui::run_width(&row.spans) <= width);
                }
                m.workflows.as_mut().unwrap().drill_in();
            }
        }
    }
}
