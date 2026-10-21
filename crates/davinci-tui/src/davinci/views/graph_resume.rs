//! Searchable, read-only picker for persisted graph executions.
//! Selecting a row never launches workers. Only an explicit resume action does.

use ratatui::text::Line;
use super::sheet::{facts, hint, Composer, SheetChrome};
use crate::davinci::model::{GraphResumeRow, Model};
use crate::davinci::ui::{section_detail, section_row, span};

pub fn visible_indices(model: &Model) -> Vec<usize> {
    let query = model.graph_resume_query.to_lowercase();
    model.graph_resume_runs.iter().enumerate().filter_map(|(index, run)| {
        ( (model.graph_resume_all || run.resumable())
            && (query.is_empty()
                || run.goal.to_lowercase().contains(&query)
                || run.run_id.to_lowercase().contains(&query)))
            .then_some(index)
    }).collect()
}

pub fn selected(model: &Model) -> Option<&GraphResumeRow> {
    let visible = visible_indices(model);
    let index = *visible.get(model.graph_resume_index.min(visible.len().saturating_sub(1)))?;
    model.graph_resume_runs.get(index)
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let visible = visible_indices(model);
    let mut rows = Vec::new();
    rows.extend(section_detail(width, th, if model.graph_resume_all {
        "All saved graph runs  ·  a: resumable only  ·  /: search"
    } else {
        "Resumable graph runs  ·  a: include completed  ·  /: search"
    }));
    if model.graph_resume_searching || !model.graph_resume_query.is_empty() {
        rows.extend(section_detail(width, th, &format!("Search: {}{}", model.graph_resume_query,
            if model.graph_resume_searching { "▌" } else { "" })));
    }
    if visible.is_empty() {
        rows.extend(section_detail(width, th, if model.graph_resume_runs.is_empty() {
            "No graph runs saved in this workspace."
        } else {
            "No runs match the current filter."
        }));
        return rows;
    }
    let selected = model.graph_resume_index.min(visible.len() - 1);
    for (position, index) in visible.into_iter().enumerate() {
        let run = &model.graph_resume_runs[index];
        let goal = run.goal.split_whitespace().collect::<Vec<_>>().join(" ");
        let goal = if goal.is_empty() { "(untitled graph)".to_owned() } else { goal };
        let status = format!("{} · {}/{} · {}",
            run.phase, run.completed, run.total, run.updated);
        rows.push(section_row(width, th, position == selected, &goal, &status));
        if position == selected && model.graph_resume_details {
            rows.extend(section_detail(width, th, &format!("Run: {}", run.run_id)));
            rows.extend(section_detail(width, th,
                &format!("Phase: {} · Tasks: {}/{} · Cost: ${:.2}",
                    run.phase, run.completed, run.total, run.cost_usd)));
            rows.extend(section_detail(width, th, if run.resumable() {
                "Press r to verify the checkpoint and resume this run."
            } else {
                "This run is complete; start a new /graph goal instead."
            }));
        }
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    SheetChrome {
        header_right: facts(th, vec![vec![span(
            format!("{} runs", model.graph_resume_runs.len()), th.muted
        )]]),
        status_third: Some(vec![span("graph recovery", th.muted)]),
        hints: vec![
            hint(th, "↑↓ select"),
            hint(th, "enter details"),
            hint(th, "r resume"),
            hint(th, "/ search"),
            hint(th, "a all"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: None,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn filter_search_and_selection_keep_real_run_id() {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
        model.graph_resume_runs = vec![
            GraphResumeRow {run_id:"run-one".into(),goal:"Fix broken login".into(),phase:"blocked".into(),..Default::default()},
            GraphResumeRow {run_id:"run-two".into(),goal:"Optimize memory".into(),phase:"done".into(),..Default::default()},
            GraphResumeRow {run_id:"run-three".into(),goal:"Improve graph recovery".into(),phase:"cancelled".into(),..Default::default()},
        ];
        assert_eq!(visible_indices(&model), vec![0,2]);
        model.graph_resume_query = "graph".into();
        assert_eq!(visible_indices(&model), vec![2]);
        assert_eq!(selected(&model).unwrap().run_id, "run-three");
        model.graph_resume_all = true;
        model.graph_resume_query.clear();
        assert_eq!(visible_indices(&model), vec![0,1,2]);
        model.graph_resume_index = 1;
        assert!(!selected(&model).unwrap().resumable());
        model.graph_resume_details = true;
        let output = lines(&model).iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        assert!(output.contains("run-two"));
        assert!(output.contains("complete"));
    }
}
