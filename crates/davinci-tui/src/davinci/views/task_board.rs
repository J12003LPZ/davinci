//! Live execution task board (`/tasks`), rendering committed execution tasks,
//! owner, activity, dependencies, evidence, and parent plan steps.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_state, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .task_board
        .as_ref()
        .filter(|sheet| !sheet.tasks.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No tasks tracked. Tasks are created during plan execution or via /tasks.",
        );
    };

    let mut rows = Vec::new();
    for task in &sheet.tasks {
        rows.extend(section_state(width, th, task.state, &task.title));
        rows.extend(section_detail(width, th, &format!("ID: {}", task.id)));
        rows.extend(section_detail(
            width,
            th,
            &format!("Status: {}", task.status),
        ));
        if let Some(owner) = &task.owner {
            rows.extend(section_detail(width, th, &format!("Owner: {owner}")));
        }
        if let Some(act) = &task.activity {
            rows.extend(section_detail(width, th, &format!("Activity: {act}")));
        }
        if !task.dependencies.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                &format!("Dependencies: {}", task.dependencies.join(", ")),
            ));
        }
        if !task.blocked_reasons.is_empty() {
            let mut blocked = section_detail(
                width,
                th,
                &format!("Blocked: {}", task.blocked_reasons.join("; ")),
            );
            for row in &mut blocked {
                for span in &mut row.spans {
                    span.style.fg = Some(th.error);
                }
            }
            rows.extend(blocked);
        }
        if !task.evidence_refs.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                &format!("Evidence: {}", task.evidence_refs.join(", ")),
            ));
        }
        if let Some(step) = &task.parent_plan_step {
            rows.extend(section_detail(width, th, &format!("Plan step: {step}")));
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let total = model
        .task_board
        .as_ref()
        .map_or(0, |sheet| sheet.tasks.len());
    let active = model.task_board.as_ref().map_or(0, |sheet| {
        sheet
            .tasks
            .iter()
            .filter(|t| t.state == State::Active)
            .count()
    });
    let done = model.task_board.as_ref().map_or(0, |sheet| {
        sheet
            .tasks
            .iter()
            .filter(|t| t.state == State::Done)
            .count()
    });

    SheetChrome {
        header_right: vec![span(format!("{total} tasks"), th.muted)],
        status_third: Some(vec![span(
            format!("{active} active · {done} done"),
            th.muted,
        )]),
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
        model::{TaskBoardRow, TaskBoardSheet},
        theme::{ColorDepth, Theme},
    };

    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        let mut row = TaskBoardRow::new(
            "task-101",
            "Build parser module",
            "in_progress",
            State::Active,
        );
        row.owner = Some("worker-1".into());
        row.dependencies = vec!["task-100".into()];
        row.parent_plan_step = Some("step-3".into());
        row.evidence_refs = vec!["evidence-abc".into()];
        row.blocked_reasons = vec!["waiting on lock".into()];
        row.activity = Some("compiling parser.rs".into());
        row.updated_at_ms = 1725800000;

        m.task_board = Some(TaskBoardSheet {
            tasks: vec![row],
            selected_index: 0,
        });
        m
    }

    #[test]
    fn test_empty_task_board() {
        let m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        let rendered = lines(&m);
        assert!(!rendered.is_empty());
        let joined: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("No tasks tracked"));
    }

    #[test]
    fn test_populated_task_board() {
        let m = model(80);
        let rendered = lines(&m);
        let joined: String = rendered
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("Build parser module"));
        assert!(joined.contains("task-101"));
        assert!(joined.contains("in_progress"));
        assert!(joined.contains("worker-1"));
        assert!(joined.contains("task-100"));
        assert!(joined.contains("waiting on lock"));
        assert!(joined.contains("evidence-abc"));
        assert!(joined.contains("step-3"));
    }

    #[test]
    fn test_chrome_metadata() {
        let m = model(80);
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("1 tasks"));
        let status = c.status_third.unwrap();
        let status_text: String = status.iter().map(|s| s.content.as_ref()).collect();
        assert!(status_text.contains("1 active · 0 done"));
    }
}
