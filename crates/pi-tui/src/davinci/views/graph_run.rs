//! `5a` — `/graph`. A task run as a graph of isolated workers, which is a
//! different instrument from `2a`: `2a` studies the code's dependencies, this
//! watches a run.
//!
//! Every worker is a child process with its own tool allowlist and its own
//! shell policy, and the screen says which policy each one got — that is the
//! whole safety argument, so it is on the row rather than in a manual. The
//! phase rail and the budgets are meters with their caps (design.md §9).
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/graph_run.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::{GraphTask, Model};
use crate::davinci::theme::{glyph, State};
use crate::davinci::ui::{
    blank, clip_ellipsis, meter, pad, span, span_strong, truncate_run, Surface, MEASURE,
};

const ID: usize = 16;
const POLICY: usize = 21;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(run) = model.graph_run.as_ref() else {
        return vec![Line::from(vec![span(
            "no graph run is in flight — /graph <goal> starts one",
            th.muted,
        )])];
    };

    let echo = Line::from(vec![
        span(format!("{} ", glyph::USER), th.primary),
        span(format!("/graph {}", run.goal), th.muted),
    ]);

    let mut rail_spans: Vec<Span<'static>> = Vec::new();
    for (name, state) in &run.phases {
        rail_spans.push(span_strong(
            format!("{} ", state.glyph()),
            th.state_color(*state),
            th,
        ));
        rail_spans.push(span(
            name.clone(),
            if *state == State::Active {
                th.text
            } else {
                th.muted
            },
        ));
        rail_spans.push(span("  ", th.border));
    }
    let rail = Line::from(rail_spans);

    let graph = Surface::new(width, th)
        .title(vec![
            span("GRAFO", th.primary),
            span(" · ", th.border),
            span("WORKER GRAPH", th.muted),
        ])
        .right(vec![
            span(format!("{} tasks", run.tasks.len()), th.muted),
            span(" · ", th.border),
            span("0 blocked", th.success),
        ])
        .rows(
            run.shape
                .iter()
                .map(|row| vec![span(row.clone(), th.border)])
                .collect(),
        )
        .lines();

    let header = Line::from(vec![
        pad(2, None),
        span(format!("{:<width$}", "WORKER", width = ID + 1), th.border),
        span(
            format!("{:<width$}", "SHELL POLICY", width = POLICY + 1),
            th.border,
        ),
        span("ARTIFACT", th.border),
    ]);

    // Below 88 the usage column goes: what a worker is doing outranks what it
    // has spent, and the run total is in the budgets below either way.
    let usage = model.width >= 88;
    let tasks: Vec<Line<'static>> = run
        .tasks
        .iter()
        .map(|task| task_row(task, model, usage))
        .collect();

    let mut cost_row = vec![
        span("cost ", th.muted),
        span(run.cost.clone(), th.text),
        span(" of ", th.muted),
        span(run.cost_cap.clone(), th.text),
        span("  ", th.border),
    ];
    cost_row.extend(meter(run.cost_fraction, 16, th, Some(th.success)));
    let budgets = vec![
        Line::from(cost_row),
        Line::from(vec![
            span("workers ", th.muted),
            span(run.workers.clone(), th.text),
            span(" · at most ", th.muted),
            span(run.parallel.clone(), th.text),
            span(" at a time", th.muted),
            span(" · revision cycles ", th.muted),
            span(run.cycles.clone(), th.text),
            span(" · replans ", th.muted),
            span(run.replans.clone(), th.text),
        ]),
        Line::from(vec![
            span("no run deadline · per-role timeouts unlimited", th.border),
            span(" · ", th.border),
            span(run.artifacts.clone(), th.border),
        ]),
    ];

    let footer = vec![
        Line::from(vec![
            span("enter open artifact", th.border),
            span(" · ", th.border),
            span("v tail a worker", th.border),
            span(" · ", th.border),
            span("r resume a stopped run", th.border),
        ]),
        Line::from(vec![
            span("a abort", th.border),
            span(" · ", th.border),
            span("esc close", th.border),
        ]),
    ];

    let mut out = vec![echo, blank(), rail, blank()];
    out.extend(graph);
    out.push(blank());
    out.push(header);
    out.extend(tasks);
    out.push(blank());
    out.extend(budgets);
    out.push(blank());
    out.extend(footer);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

fn task_row(task: &GraphTask, model: &Model, usage: bool) -> Line<'static> {
    let th = &model.theme;
    let glyph = if task.state == State::Active {
        model.theme.spinner(model.tick, model.animate).to_string()
    } else {
        task.state.glyph().to_string()
    };
    let color = th.state_color(task.state);
    let text_color = if task.state == State::Active {
        th.text
    } else {
        th.muted
    };
    let detail = if task.state == State::Queued {
        th.border
    } else {
        th.muted
    };

    let mut spans = vec![
        span_strong(format!("{glyph} "), color, th),
        span(format!("{:<width$}", task.id, width = ID + 1), text_color),
        span(
            format!("{:<width$}", task.policy, width = POLICY + 1),
            detail,
        ),
        span(format!("{:<29}", clip_ellipsis(&task.artifact, 28)), detail),
    ];
    if usage {
        spans.push(span(task.usage.clone(), th.border));
    }
    Line::from(spans)
}

/// The sheet's frame (design.md §11). Filled in per artboard.
pub fn chrome(model: &Model) -> crate::davinci::views::sheet::SheetChrome {
    let _ = model;
    crate::davinci::views::sheet::SheetChrome::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::GraphRunSheet;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.graph_run = Some(GraphRunSheet {
            goal: "add prompt-cache parity to the openai adapter --complex".into(),
            phases: vec![
                ("classify".into(), State::Done),
                ("implement".into(), State::Active),
                ("verify".into(), State::Queued),
            ],
            shape: vec![
                "t1 classifier ─┬─ t2 researcher ─┐".into(),
                "               └─ t6 writer ◉".into(),
            ],
            tasks: vec![
                GraphTask {
                    id: "t1 classifier".into(),
                    policy: "read-only".into(),
                    artifact: "feature · complex".into(),
                    usage: "2.1k↑ 0.4k↓ $0.01 4s".into(),
                    state: State::Done,
                },
                GraphTask {
                    id: "t6 writer".into(),
                    policy: "write-no-git-mutation".into(),
                    artifact: "davinci-ai\\src\\openai.rs".into(),
                    usage: "64k↑ 9.7k↓ $0.71 2m14s".into(),
                    state: State::Active,
                },
                GraphTask {
                    id: "t7 reviewer".into(),
                    policy: "read-and-test".into(),
                    artifact: "pending · waits on t6".into(),
                    usage: "—".into(),
                    state: State::Queued,
                },
            ],
            cost: "$1.31".into(),
            cost_cap: "$8.00".into(),
            cost_fraction: 0.16,
            workers: "6 of 12".into(),
            parallel: "3".into(),
            cycles: "0 of 2".into(),
            replans: "0 of 1".into(),
            artifacts: ".davinci\\graph\\g-7f2a\\".into(),
        });
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_worker_states_its_shell_policy_on_its_own_row() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("SHELL POLICY")));
        assert!(rows
            .iter()
            .any(|row| row.contains("t6 writer") && row.contains("write-no-git-mutation")));
        assert!(rows
            .iter()
            .any(|row| row.contains("t7 reviewer") && row.contains("read-and-test")));
    }

    #[test]
    fn the_cost_meter_carries_its_cap() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("$1.31") && row.contains("$8.00")));
    }

    #[test]
    fn below_88_columns_the_usage_column_goes() {
        let wide = model(100);
        let rows: Vec<String> = lines(&wide).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("2m14s")));

        let narrow = model(80);
        let rows: Vec<String> = lines(&narrow).iter().map(text).collect();
        assert!(!rows.iter().any(|row| row.contains("2m14s")));
    }

    #[test]
    fn only_the_active_task_animates_and_stillness_freezes_it() {
        let mut m = model(100);
        let frame = |m: &Model| -> Vec<String> { lines(m).iter().map(text).collect() };
        m.tick = 0;
        let first = frame(&m);
        m.tick = 1;
        let second = frame(&m);
        let moved: Vec<usize> = first
            .iter()
            .zip(&second)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(moved.len(), 1, "exactly one row animates: {moved:?}");
        assert!(first[moved[0]].contains("t6 writer"));

        m.animate = false;
        m.tick = 0;
        let still_first = frame(&m);
        m.tick = 1;
        let still_second = frame(&m);
        assert_eq!(still_first, still_second, "stillness froze nothing");
    }

    #[test]
    fn with_no_run_in_flight_the_screen_says_so() {
        let mut m = model(100);
        m.graph_run = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("no graph run is in flight"));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
