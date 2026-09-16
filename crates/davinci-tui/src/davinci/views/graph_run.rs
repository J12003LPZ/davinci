//! Read-only graph-run progress. Worker state is not keyboard focus;
//! policies, artifacts and usage come from the actual run snapshot.

use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::ui::{self, section_detail, section_heading, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    lines_in(model, model.height.saturating_sub(3))
}

pub fn lines_in(model: &Model, height: u16) -> Vec<Line<'static>> {
    let Some(run) = &model.graph_run else {
        return structured_lines(model);
    };
    let layout = super::graph_layout::layout_graph(
        run,
        &model.graph_canvas,
        model.width,
        height.saturating_sub(2),
    );
    if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
        let mut rows = Vec::new();
        for issue in &layout.issues {
            rows.extend(section_detail(model.width, &model.theme, issue));
        }
        rows.extend(structured_lines(model));
        return rows;
    }
    let done = run.tasks.iter().filter(|t| t.state == State::Done).count();
    let mut rows = vec![
        Line::from(ui::truncate_run(
            vec![span(
                format!(
                    "{} · {} · Follow {} · {:?}",
                    run.id,
                    run.lifecycle,
                    if model.graph_canvas.follow_live {
                        "on"
                    } else {
                        "off"
                    },
                    model.graph_canvas.view_mode
                ),
                model.theme.primary,
            )],
            model.width,
        )),
        Line::from(ui::truncate_run(
            vec![span(
                format!(
                    "{done}/{} workers complete · {} · {} / {}",
                    run.tasks.len(),
                    run.elapsed,
                    run.cost,
                    run.cost_cap
                ),
                model.theme.muted,
            )],
            model.width,
        )),
    ];
    rows.extend(super::graph_canvas::lines(
        model,
        &layout,
        (model.tick % 4) as u8,
    ));
    rows
}

pub fn structured_lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(run) = &model.graph_run else {
        return section_detail(
            width,
            th,
            "No graph run to show. Use /graph <goal> from the conversation to start one.",
        );
    };
    let done = run
        .tasks
        .iter()
        .filter(|task| task.state == State::Done)
        .count();
    let mut rows = section_heading(width, th, &run.goal);
    rows.extend(section_detail(
        width,
        th,
        &format!("{done} of {} workers complete", run.tasks.len()),
    ));
    if !run.lifecycle.is_empty() {
        let label = match run.lifecycle.as_str() {
            "pause_requested" => "pause_requested · waiting for safe boundary",
            "paused" => "paused",
            "stop_requested" => "stop_requested",
            "stopped" => "stopped",
            "recovery_required" => "recovery_required",
            other => other,
        };
        rows.extend(section_detail(width, th, &format!("Lifecycle: {label}")));
    }
    if let Some(status) = &run.control_status {
        rows.extend(section_detail(width, th, &format!("Control: {status}")));
    }
    for (phase, state) in &run.phases {
        rows.extend(section_state(width, th, *state, phase));
    }
    if !run.shape.is_empty() {
        rows.extend(section_heading(width, th, "Worker graph"));
        for line in &run.shape {
            // Keep the authored connector grid; the worker list below provides
            // full names and paths when the diagram is too wide.
            rows.push(Line::from(ui::truncate_run(
                vec![span(format!("   {line}"), th.muted)],
                width,
            )));
        }
    }
    if run.tasks.is_empty() {
        rows.extend(section_detail(width, th, "No workers reported yet."));
    }
    for task in &run.tasks {
        let is_selected = run.selected_node_id.as_deref() == Some(&task.id);
        if is_selected {
            rows.push(ui::section_row(
                width,
                th,
                true,
                &format!("{} {}", task.state.glyph(), task.id),
                &task.usage,
            ));
        } else {
            rows.extend(section_state(width, th, task.state, &task.id));
        }
        for (label, value) in [
            ("Policy", &task.policy),
            ("Artifact", &task.artifact),
            ("Usage", &task.usage),
        ] {
            if !value.is_empty() {
                rows.extend(section_detail(width, th, &format!("{label}: {value}")));
            }
        }
        if is_selected && run.inspecting_node {
            if !task.role.is_empty() {
                rows.extend(section_detail(width, th, &format!("Role: {}", task.role)));
            }
            if !task.dependencies.is_empty() {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Dependencies: {}", task.dependencies.join(", ")),
                ));
            }
            if !task.owner.is_empty() {
                rows.extend(section_detail(width, th, &format!("Owner: {}", task.owner)));
            }
            if task.attempts > 0 {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Attempts: {}", task.attempts),
                ));
            }
            if let Some(err) = &task.error {
                rows.extend(section_detail(width, th, &format!("Error: {err}")));
            }
            if let Some(contract) = &task.public_contract {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Public Contract: {contract}"),
                ));
            }
            for tool in &task.recent_tools {
                rows.extend(section_detail(width, th, &format!("Recent Tool: {tool}")));
            }
        }
    }
    let cost = if run.cost_cap.is_empty() {
        run.cost.clone()
    } else {
        format!("{} of {}", run.cost, run.cost_cap)
    };
    for (label, value) in [
        ("Run", &run.id),
        ("Mode", &run.mode),
        ("Milestone", &run.milestone),
        ("Cost", &cost),
        ("Workers", &run.workers),
        ("Parallel limit", &run.parallel),
        ("Revision cycles", &run.cycles),
        ("Replans", &run.replans),
        ("Elapsed", &run.elapsed),
        ("Artifacts", &run.artifacts),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    for note in &run.ecosystem {
        rows.extend(section_detail(width, th, note));
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let run = model.graph_run.as_ref();
    SheetChrome {
        header_right: facts(
            th,
            run.map(|run| {
                [&run.id, &run.mode, &run.milestone]
                    .into_iter()
                    .filter(|value| !value.is_empty())
                    .map(|value| vec![span(value.clone(), th.muted)])
                    .collect()
            })
            .unwrap_or_default(),
        ),
        status_third: run
            .and_then(|run| run.phases.iter().find(|(_, state)| *state == State::Active))
            .map(|(phase, _)| vec![span(phase.clone(), th.muted)]),
        status_right: run
            .filter(|run| !run.cost.is_empty() && !run.cost_cap.is_empty())
            .map(|run| status_meter(th, "run cost", run.cost_fraction, &run.cost, &run.cost_cap)),
        hints: vec![
            hint(th, "↑↓ select"),
            hint(th, "enter inspect"),
            hint(th, "p pause/resume"),
            hint(th, "x stop"),
            hint(th, "r retry"),
            hint(th, "d diff"),
        ],
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
        theme::{ColorDepth, Theme},
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress_screen(&mut m, "5a");
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
    fn structured_fallback_retains_every_real_policy_artifact_and_usage() {
        let m = model(120);
        let drawn = structured_lines(&m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let run = m.graph_run.as_ref().unwrap();
        for task in &run.tasks {
            for value in [&task.id, &task.policy, &task.artifact, &task.usage] {
                assert!(drawn.contains(value), "{value}");
            }
        }
        for value in [&run.cost, &run.cost_cap, &run.artifacts, &run.elapsed] {
            assert!(drawn.contains(value));
        }
        assert!(ui::focused_row(&structured_lines(&m)).is_none());
        assert!(!drawn.contains("each one a child process") && !drawn.contains("ctrl+c aborts"));
    }
    #[test]
    fn progress_counts_completed_workers_and_does_not_claim_a_missing_cost_cap() {
        let mut m = model(80);
        let run = m.graph_run.as_mut().unwrap();
        run.cost_cap.clear();
        assert!(chrome(&m).status_right.is_none());
        assert!(text(&m).contains("workers complete"));
    }
    #[test]
    fn unavailable_and_narrow_runs_are_readable() {
        let mut m = model(80);
        m.graph_run = None;
        assert!(text(&m).contains("No graph run"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }

    #[test]
    fn test_no_node_selected() {
        let m = model(80);
        assert!(m.graph_run.as_ref().unwrap().selected_node_id.is_none());
        assert!(ui::focused_row(&lines(&m)).is_none());
    }

    #[test]
    fn test_node_selection_and_inspection() {
        let mut m = model(80);
        let run = m.graph_run.as_mut().unwrap();
        run.selected_node_id = Some("t1 classifier".into());
        run.inspecting_node = true;
        let task = &mut run.tasks[0];
        task.role = "classifier".into();
        task.public_contract = Some("inputs: user prompt, outputs: Classification".into());
        task.recent_tools = vec!["read(crates/davinci-ai/src/openai.rs)".into()];

        assert!(ui::focused_row(&structured_lines(&m)).is_some());
        let drawn = structured_lines(&m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("Contract: inputs: user prompt"));
        assert!(drawn.contains("Recent Tool: read"));
    }

    #[test]
    fn test_one_second_refresh_while_selected_node_finishes() {
        let mut m = model(80);
        m.graph_run.as_mut().unwrap().selected_node_id = Some("t6 writer".into());
        // Simulating refresh where t6 writer finishes
        let run = m.graph_run.as_mut().unwrap();
        let writer = run.tasks.iter_mut().find(|t| t.id == "t6 writer").unwrap();
        writer.state = State::Done;
        writer.artifact = "patch committed".into();

        // Selection is maintained on t6 writer
        assert_eq!(
            m.graph_run.as_ref().unwrap().selected_node_id.as_deref(),
            Some("t6 writer")
        );
        let drawn = text(&m);
        assert!(drawn.contains("t6 writer"));
        assert!(drawn.contains("patch committed"));
    }

    #[test]
    fn test_forty_column_screen() {
        let mut m = model(40);
        m.graph_run.as_mut().unwrap().selected_node_id = Some("t2 researcher".into());
        m.graph_run.as_mut().unwrap().inspecting_node = true;
        for row in lines(&m) {
            assert!(ui::run_width(&row.spans) <= 40);
        }
    }

    #[test]
    fn test_long_artifacts() {
        let mut m = model(80);
        let long_artifact = "a".repeat(400);
        m.graph_run.as_mut().unwrap().tasks[0].artifact = long_artifact.clone();
        let drawn = text(&m);
        assert!(drawn.contains("a"));
        for row in lines(&m) {
            assert!(ui::run_width(&row.spans) <= 80);
        }
    }

    #[test]
    fn test_pause_label_requested_until_ack() {
        let mut m = model(80);
        m.graph_run.as_mut().unwrap().lifecycle = "pause_requested".into();
        let drawn_requested = text(&m);
        assert!(drawn_requested.contains("pause_requested"));

        m.graph_run.as_mut().unwrap().lifecycle = "paused".into();
        let drawn_paused = text(&m);
        assert!(drawn_paused.contains("paused"));
    }

    #[test]
    fn test_hidden_reasoning_never_displayed() {
        let mut m = model(80);
        m.graph_run.as_mut().unwrap().selected_node_id = Some("t1 classifier".into());
        m.graph_run.as_mut().unwrap().inspecting_node = true;
        let drawn = text(&m);
        assert!(!drawn.contains("thinking:"));
        assert!(!drawn.contains("reasoning:"));
        assert!(!drawn.contains("<thought>"));
    }
}
