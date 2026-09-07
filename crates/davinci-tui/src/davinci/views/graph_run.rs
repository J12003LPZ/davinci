//! Read-only graph-run progress. Worker state is not keyboard focus;
//! policies, artifacts and usage come from the actual run snapshot.

use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::ui::{self, section_detail, section_heading, section_state, span};
use crate::davinci::{model::Model, theme::State};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
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
        rows.extend(section_state(width, th, task.state, &task.id));
        for (label, value) in [
            ("Policy", &task.policy),
            ("Artifact", &task.artifact),
            ("Usage", &task.usage),
        ] {
            if !value.is_empty() {
                rows.extend(section_detail(width, th, &format!("{label}: {value}")));
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
        hints: vec![hint(th, "↑↓ scroll")],
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
    fn every_worker_retains_its_real_policy_artifact_and_usage() {
        let m = model(120);
        let drawn = text(&m);
        let run = m.graph_run.as_ref().unwrap();
        for task in &run.tasks {
            for value in [&task.id, &task.policy, &task.artifact, &task.usage] {
                assert!(drawn.contains(value), "{value}");
            }
        }
        for value in [&run.cost, &run.cost_cap, &run.artifacts, &run.elapsed] {
            assert!(drawn.contains(value));
        }
        assert!(ui::focused_row(&lines(&m)).is_none());
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
}
