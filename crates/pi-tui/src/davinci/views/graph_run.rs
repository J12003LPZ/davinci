//! `5a` — `/graph`. A task run as a graph of isolated workers, which is a
//! different instrument from `2a`: `2a` studies the code's dependencies, this
//! watches a run.
//!
//! Every worker is a child process with its own tool allowlist and its own
//! shell policy, and the screen says which policy each one got — that is the
//! whole safety argument, so it is on the row rather than in a manual. The
//! stage strip and the budgets are meters with their caps (design.md §9).
//!
//! Mirrors artboard `5a` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::model::{GraphTask, Model};
use crate::davinci::theme::State;
use crate::davinci::ui::{
    blank, clip_ellipsis, column_header, footnote, meter, run_width, selection_bar, span,
    span_strong, spread, truncate_run, Surface,
};

const ID: u16 = 16;
const POLICY: u16 = 21;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(run) = model.graph_run.as_ref() else {
        return vec![Line::from(vec![span(
            "no graph run is in flight — /graph <goal> starts one",
            th.muted,
        )])];
    };

    // The stage strip: every phase with its glyph, joined by connectors, the
    // elapsed time at the right edge.
    let mut strip: Vec<Span<'static>> = Vec::new();
    for (index, (name, state)) in run.phases.iter().enumerate() {
        if index > 0 {
            strip.push(span(" ── ", th.border));
        }
        strip.push(span_strong(
            format!("{} ", state.glyph()),
            th.state_color(*state),
            th,
        ));
        strip.push(span(
            name.clone(),
            if *state == State::Active {
                th.text
            } else {
                th.muted
            },
        ));
    }
    let elapsed = if run.elapsed.is_empty() {
        Vec::new()
    } else {
        vec![span(format!("{} elapsed", run.elapsed), th.border)]
    };
    let rail = spread(width, strip, elapsed);

    let blocked = run
        .tasks
        .iter()
        .filter(|task| task.state == State::Failed)
        .count();
    let graph = Surface::new(width, th)
        .title(vec![
            span("GRAFO", th.primary),
            span(" · ", th.border),
            span("WORKER GRAPH", th.muted),
        ])
        .right(vec![
            span(format!("{} tasks", run.tasks.len()), th.muted),
            span(" · ", th.border),
            span(format!("{} parallel", run.parallel), th.muted),
            span(" · ", th.border),
            span(
                format!("{blocked} blocked"),
                if blocked > 0 { th.warning } else { th.success },
            ),
        ])
        .rows(
            run.shape
                .iter()
                .map(|row| vec![span(row.clone(), th.border)])
                .collect(),
        )
        .lines();

    // Below 88 the usage column goes: what a worker is doing outranks what it
    // has spent, and the run total is in the budgets below either way.
    let usage = model.width >= 88;
    let mut columns: Vec<(&str, u16, bool)> = vec![
        ("", LEAD - 1, false),
        ("WORKER", ID, false),
        ("POLICY", POLICY, false),
        ("ARTIFACT", 0, false),
    ];
    if usage {
        columns.push(("↑ ↓ $ TIME", 24, true));
    }
    let header = column_header(width, &columns, th);
    let tasks: Vec<Line<'static>> = run
        .tasks
        .iter()
        .map(|task| task_row(task, model, usage, width))
        .collect();

    let mut cost_row = vec![
        span("cost ", th.muted),
        span(run.cost.clone(), th.text),
        span(" of ", th.muted),
        span(run.cost_cap.clone(), th.text),
        span("  ", th.border),
    ];
    cost_row.extend(meter(run.cost_fraction, 16, th, Some(th.success)));
    let mut budgets = vec![
        Line::from(cost_row),
        Line::from(vec![
            span("workers ", th.muted),
            span(run.workers.clone(), th.text),
            span(" · at most ", th.muted),
            span(run.parallel.clone(), th.text),
            span(" at a time", th.muted),
        ]),
        Line::from(vec![
            span("revision cycles ", th.muted),
            span(run.cycles.clone(), th.text),
            span(" · replans ", th.muted),
            span(run.replans.clone(), th.text),
        ]),
        Line::from(vec![span(
            "no run deadline · per-role timeouts unlimited",
            th.border,
        )]),
    ];
    budgets.extend(footnote(
        width,
        vec![
            span("artifacts in ", th.muted),
            span(run.artifacts.clone(), th.secondary),
        ],
        vec![span(
            "ctrl+c aborts the run, keeps the artifacts",
            th.border,
        )],
        th,
    ));

    let mut out = vec![rail, blank()];
    out.extend(graph);
    out.push(blank());
    out.push(Line::from(vec![
        span("workers", th.text),
        span(
            "   each one a child process · own tool allowlist · own bash policy",
            th.border,
        ),
    ]));
    out.extend(header);
    out.extend(tasks);
    out.push(blank());
    out.extend(budgets);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// The sheet's frame (design.md §11): the run in the header, the phase in
/// hand in the status bar with the run cost as its meter.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let run = model.graph_run.as_ref();
    let phase = run.and_then(|run| {
        run.phases
            .iter()
            .find(|(_, state)| *state == State::Active)
            .map(|(name, _)| name.clone())
    });
    SheetChrome {
        header_right: facts(
            th,
            vec![
                run.filter(|r| !r.id.is_empty())
                    .map(|r| vec![span("run ", th.muted), span(r.id.clone(), th.text)])
                    .unwrap_or_default(),
                run.filter(|r| !r.mode.is_empty())
                    .map(|r| vec![span(r.mode.clone(), th.muted)])
                    .unwrap_or_default(),
                run.filter(|r| !r.milestone.is_empty())
                    .map(|r| vec![span(format!("milestone {}", r.milestone), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: phase.map(|phase| vec![span(phase, th.primary)]),
        status_right: run
            .map(|r| status_meter(th, "run cost", r.cost_fraction, &r.cost, &r.cost_cap)),
        hints: vec![
            hint(th, "enter open artifact"),
            hint(th, "v tail a worker"),
            hint(th, "r resume a stopped run"),
            hint(th, "a abort"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("/graph-view t6"),
        echo: run.map(|r| format!("/graph {}", r.goal)),
    }
}

fn task_row(task: &GraphTask, model: &Model, usage: bool, width: u16) -> Line<'static> {
    let th = &model.theme;
    let active = task.state == State::Active;
    let glyph = if active {
        model.theme.spinner(model.tick, model.animate).to_string()
    } else {
        task.state.glyph().to_string()
    };
    let color = th.state_color(task.state);
    let text_color = if active { th.text } else { th.muted };
    let detail = if task.state == State::Queued {
        th.border
    } else {
        th.muted
    };

    let mut spans = vec![
        selection_bar(active, th),
        span_strong(format!("{glyph} "), color, th),
        span(format!("{:<w$} ", task.id, w = ID as usize), text_color),
        span(format!("{:<w$} ", task.policy, w = POLICY as usize), detail),
    ];
    let right = if usage {
        vec![span(task.usage.clone(), th.border)]
    } else {
        Vec::new()
    };
    let room = width
        .saturating_sub(run_width(&spans))
        .saturating_sub(run_width(&right))
        .saturating_sub(1);
    spans.push(span(clip_ellipsis(&task.artifact, room), detail));
    spread(width, spans, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.graph_run = Some(fixtures::graph_run_sheet());
        model.toggle_screen(Screen::GraphRun);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_stage_strip_joins_every_phase_and_states_the_elapsed_time() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(
            rows[0].starts_with("✓ classify ── ✓ investigate ── ✓ plan ── ◉ implement"),
            "{}",
            rows[0]
        );
        assert!(rows[0].trim_end().ends_with("6m18s elapsed"), "{}", rows[0]);
    }

    #[test]
    fn every_worker_states_its_policy_artifact_and_spend() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("7 tasks · 3 parallel · 0 blocked")));
        assert!(rows
            .iter()
            .any(|row| row.contains("WORKER") && row.contains("POLICY")));
        let writer = rows
            .iter()
            .find(|row| row.contains("t6 writer") && row.contains("write-no-git"))
            .expect("the writer row");
        assert!(
            writer.starts_with("▌  "),
            "the running worker is marked: {writer}"
        );
        assert!(writer.contains("write-no-git-mutation"), "{writer}");
        assert!(
            writer.trim_end().ends_with("64k↑ 9.7k↓ $0.71 2m14s"),
            "{writer}"
        );
        assert!(rows.iter().any(|row| row.contains("cost $1.31 of $8.00")));
        assert!(rows
            .iter()
            .any(|row| row.contains("artifacts in .pi\\graph\\g-7f2a\\")
                && row.contains("ctrl+c aborts the run, keeps the artifacts")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn below_eighty_eight_columns_the_spend_column_goes() {
        let m = model(80);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(!rows.iter().any(|row| row.contains("$0.71")));
        assert!(rows.iter().any(|row| row.contains("t6 writer")));
    }

    #[test]
    fn no_run_says_how_to_start_one() {
        let mut m = model(100);
        m.graph_run = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("/graph <goal> starts one")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "5a");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "run g-7f2a │ complex │ milestone 2 of 4");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "implement");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(right.starts_with("run cost "), "{right}");
        assert!(right.ends_with(" $1.31/$8.00"), "{right}");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Prompt("/graph-view t6"));
        assert_eq!(
            c.echo.as_deref(),
            Some("/graph add prompt-cache parity to the openai adapter --complex")
        );
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("enter open artifact │ v tail a worker"),
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
