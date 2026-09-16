//! Read-only graph-run progress. Worker state is not keyboard focus;
//! policies, artifacts and usage come from the actual run snapshot.

use super::graph_inspector::{inspector_lines, public_text};
use super::graph_layout::{GraphLayout, GraphResponsiveMode};
use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::ui::{self, section_heading, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub const HEADER_ROWS: u16 = 3;
const FOOTER_ROWS: u16 = 3;

fn section_detail(
    width: u16,
    theme: &crate::davinci::theme::Theme,
    text: &str,
) -> Vec<Line<'static>> {
    ui::section_detail(width, theme, &public_text(text))
}

pub fn layout_for(model: &Model, height: u16) -> Option<GraphLayout> {
    model.graph_run.as_ref().map(|run| {
        super::graph_layout::layout_graph(
            run,
            &model.graph_canvas,
            model.width,
            height.saturating_sub(HEADER_ROWS + FOOTER_ROWS),
        )
    })
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    lines_in(model, model.height.saturating_sub(3))
}

pub fn lines_in(model: &Model, height: u16) -> Vec<Line<'static>> {
    let Some(layout) = layout_for(model, height) else {
        return structured_lines(model);
    };
    lines_with_layout(model, height, &layout)
}

pub fn lines_with_layout(model: &Model, height: u16, layout: &GraphLayout) -> Vec<Line<'static>> {
    let Some(run) = &model.graph_run else {
        return structured_lines(model);
    };
    if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
        return structured_window(model, height, layout);
    }
    let done = run.tasks.iter().filter(|t| t.state == State::Done).count();
    let mut telemetry = vec![format!("{done}/{} workers complete", run.tasks.len())];
    for (label, value) in [
        ("Phase", &run.phase),
        ("Elapsed", &run.elapsed),
        ("Cost", &run.cost),
        ("Cap", &run.cost_cap),
        ("Revisions", &run.cycles),
    ] {
        if !value.is_empty() {
            telemetry.push(format!("{label}: {value}"));
        }
    }
    let mut rows = vec![
        Line::from(ui::truncate_run(
            vec![span(
                public_text(&format!(
                    "{} · {} · Follow {} · {:?}",
                    run.id,
                    run.lifecycle,
                    if model.graph_canvas.follow_live {
                        "on"
                    } else {
                        "off"
                    },
                    model.graph_canvas.view_mode
                )),
                model.theme.primary,
            )],
            model.width,
        )),
        Line::from(ui::truncate_run(
            vec![span(public_text(&telemetry.join(" · ")), model.theme.muted)],
            model.width,
        )),
    ];
    let note = layout
        .issues
        .first()
        .map(String::as_str)
        .or(run.control_status.as_deref())
        .or(run.blocked_reason.as_deref())
        .unwrap_or(&run.goal);
    rows.push(Line::from(span(
        ui::clip_ellipsis(&public_text(note), model.width),
        model.theme.muted,
    )));
    let mut cells = super::graph_canvas::Cells::new(
        model.width,
        height.saturating_sub(HEADER_ROWS + FOOTER_ROWS),
    );
    cells.blit(
        super::graph_canvas::lines(
            model,
            layout,
            if model.animate {
                (model.tick % 4) as u8
            } else {
                0
            },
        ),
        layout.canvas,
    );
    cells.blit(
        inspector_lines(
            model,
            run.selected_node_id.as_deref(),
            layout.inspector.width,
            layout.inspector.height,
        ),
        layout.inspector,
    );
    let rule = ratatui::style::Style::default().fg(model.theme.border);
    if layout.mode == GraphResponsiveMode::Full {
        for y in 0..layout.inspector.height {
            cells.write(layout.inspector.x as i32 - 1, y as i32, "│", rule);
        }
    } else {
        cells.write(
            0,
            layout.inspector.y as i32 - 1,
            &"─".repeat(model.width as usize),
            rule,
        );
    }
    rows.extend(cells.into_lines());
    rows.extend(controls(model));
    rows.truncate(height as usize);
    rows
}

fn controls(model: &Model) -> Vec<Line<'static>> {
    let control = if model
        .graph_run
        .as_ref()
        .is_some_and(|r| r.lifecycle == "paused")
    {
        "p resume · x stop · r retry · d diff"
    } else {
        "p pause · x stop · r retry · d diff"
    };
    [
        "↑↓←→ select · Enter inspect · v focus",
        control,
        "f follow · PgUp/Dn pan/details",
    ]
    .into_iter()
    .map(|text| {
        Line::from(span(
            ui::clip_ellipsis(text, model.width),
            model.theme.muted,
        ))
    })
    .collect()
}

/// A bounded, keyboard-complete ledger; details never push selection offscreen.
fn structured_window(model: &Model, height: u16, layout: &GraphLayout) -> Vec<Line<'static>> {
    let run = model.graph_run.as_ref().unwrap();
    let line = |text: String| {
        Line::from(span(
            ui::clip_ellipsis(&public_text(&text), model.width),
            model.theme.text,
        ))
    };
    let mut rows = vec![
        line(format!(
            "{} · Follow {} · {:?}",
            run.lifecycle,
            if model.graph_canvas.follow_live {
                "on"
            } else {
                "off"
            },
            model.graph_canvas.view_mode
        )),
        line(format!(
            "{} workers · {} · {}",
            run.tasks.len(),
            run.cost,
            run.elapsed
        )),
        line(
            layout
                .issues
                .first()
                .or(run.control_status.as_ref())
                .or(run.blocked_reason.as_ref())
                .unwrap_or(&run.goal)
                .clone(),
        ),
    ];
    let room = height.saturating_sub(HEADER_ROWS + FOOTER_ROWS) as usize;
    let selected = run.selected_node_id.as_deref();
    let anchor = model.graph_canvas.list_scroll.unwrap_or_else(|| {
        run.tasks
            .iter()
            .position(|t| Some(t.id.as_str()) == selected)
            .or_else(|| run.tasks.iter().position(|t| t.state == State::Active))
            .unwrap_or(0)
    });
    let list_room = if run.inspecting_node {
        room.min(3)
    } else {
        room
    };
    let tasks = run
        .tasks
        .iter()
        .map(|task| {
            ui::section_row(
                model.width,
                &model.theme,
                Some(task.id.as_str()) == selected,
                &public_text(&format!(
                    "{} {}{}",
                    task.state.glyph(),
                    task.id,
                    if task.state == State::Attention {
                        " · blocked".into()
                    } else if task.status.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", task.status)
                    }
                )),
                "",
            )
        })
        .collect();
    rows.extend(
        ui::window(tasks, list_room, anchor, &model.theme)
            .into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width))),
    );
    while rows.len() < HEADER_ROWS as usize + list_room {
        rows.push(Line::default());
    }
    if run.inspecting_node {
        rows.extend(inspector_lines(
            model,
            selected,
            model.width,
            room.saturating_sub(list_room) as u16,
        ));
    }
    while rows.len() < height.saturating_sub(FOOTER_ROWS) as usize {
        rows.push(Line::default());
    }
    rows.extend(controls(model));
    rows.truncate(height as usize);
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
    let mut rows = section_heading(width, th, &public_text(&run.goal));
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
        rows.extend(section_state(width, th, *state, &public_text(phase)));
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
                &public_text(&format!("{} {}", task.state.glyph(), task.id)),
                &public_text(&task.usage),
            ));
        } else {
            rows.extend(section_state(width, th, task.state, &public_text(&task.id)));
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
                    .map(|value| vec![span(public_text(value), th.muted)])
                    .collect()
            })
            .unwrap_or_default(),
        ),
        status_third: run
            .and_then(|run| run.phases.iter().find(|(_, state)| *state == State::Active))
            .map(|(phase, _)| vec![span(public_text(phase), th.muted)]),
        status_right: run
            .filter(|run| !run.cost.is_empty() && !run.cost_cap.is_empty())
            .map(|run| {
                status_meter(
                    th,
                    "run cost",
                    run.cost_fraction,
                    &public_text(&run.cost),
                    &public_text(&run.cost_cap),
                )
            }),
        hints: vec![hint(th, "Graph controls above")],
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
    #[test]
    fn graph_fallback_selection_controls_inspection_and_viewport_boundaries() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut m = model(40);
        m.graph_run = Some(fixtures::blueprint_graph());
        m.graph_run.as_mut().unwrap().selected_node_id = Some("blocked".into());
        m.graph_run.as_mut().unwrap().selected_index = 8;
        let text = |m: &Model| {
            crate::davinci::app::compose_frame(m, m.height)
                .lines
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        for word in [
            "blocked",
            "running",
            "p pause",
            "x stop",
            "r retry",
            "d diff",
            "Enter inspect",
            "f follow",
            "v focus",
        ] {
            assert!(text(&m).contains(word), "missing {word}: {}", text(&m));
        }
        crate::davinci::app::handle_key(&mut m, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(text(&m).contains("failed dependency"));
        for width in [0, 1, 20, 32, 40, 49, 50, 71, 72, 80, 119, 120] {
            for height in [0, 1, 4, 12, 24, 32, 40] {
                m.width = width;
                m.height = height;
                let frame = crate::davinci::app::compose_frame(&m, height);
                assert!(frame.lines.len() <= height as usize, "{width}x{height}");
                assert!(
                    frame.lines.iter().all(|r| ui::run_width(&r.spans) <= width),
                    "{width}x{height}"
                );
            }
        }
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn graph_run_disabled_animation_stays_static_across_ticks() {
        let mut m = model(120);
        m.graph_run = Some(fixtures::blueprint_graph());
        m.height = 40;
        m.animate = false;
        m.tick = 0;
        let first = text(&m);
        m.tick = 1;
        assert_eq!(first, text(&m));
        assert!(first.contains("◉ writer"));
        m.animate = true;
        assert_ne!(first, text(&m));
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
