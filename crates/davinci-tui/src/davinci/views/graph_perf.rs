//! Explicit local performance fixture; no timing thresholds in normal CI.
use super::*;
use crate::davinci::views::{
    graph_layout::{layout_graph, GraphLayout},
    graph_run,
};
use crate::davinci::{
    model::{GraphTask, Model, Screen},
    theme::{ColorDepth, Theme},
};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::{
    hint::black_box,
    mem::{size_of, size_of_val},
    time::Instant,
};

fn distribution(mut samples: Vec<u128>) -> serde_json::Value {
    samples.sort_unstable();
    let quantile =
        |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
    serde_json::json!({"p50_us":quantile(50),"p95_us":quantile(95),"p99_us":quantile(99)})
}
fn measured<T>(mut action: impl FnMut() -> T) -> serde_json::Value {
    distribution(
        (0..32)
            .map(|_| {
                let started = Instant::now();
                black_box(action());
                started.elapsed().as_micros()
            })
            .collect(),
    )
}
fn owned_bytes(layout: &GraphLayout) -> usize {
    size_of_val(layout)
        + layout
            .nodes
            .iter()
            .map(|node| {
                size_of_val(node)
                    + node.id.capacity()
                    + node.members.capacity() * size_of::<String>()
                    + node.members.iter().map(String::capacity).sum::<usize>()
            })
            .sum::<usize>()
        + layout
            .edges
            .iter()
            .map(|edge| size_of_val(edge) + edge.points.capacity() * size_of::<(u16, u16)>())
            .sum::<usize>()
        + layout
            .issues
            .iter()
            .map(|issue| size_of_val(issue) + issue.capacity())
            .sum::<usize>()
}

#[test]
#[ignore = "measured graph layout/render/navigation fixture; run with --ignored --nocapture"]
fn graph_dag_performance() {
    for count in [25usize, 100, 500, 1000] {
        let tasks: Vec<_> = (0..count)
            .map(|i| {
                let mut dependencies = Vec::new();
                for predecessor in [
                    i.checked_sub(1),
                    i.checked_sub(7),
                    (i >= 13).then_some(i / 2),
                ]
                .into_iter()
                .flatten()
                {
                    dependencies.push(format!("n{predecessor:04}"));
                }
                dependencies.sort();
                dependencies.dedup();
                GraphTask {
                    id: format!("n{i:04}"),
                    status: "ready".into(),
                    phase: format!("stage-{}", i / 25),
                    artifact: "bench".into(),
                    usage: "0t".into(),
                    state: if i == 0 { State::Active } else { State::Queued },
                    role: "worker".into(),
                    dependencies,
                    ..Default::default()
                }
            })
            .collect();
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 160, 48, false);
        model.screen = Screen::GraphRun;
        model.graph_canvas.follow_live = false;
        model.graph_canvas.node_order = tasks.iter().map(|task| task.id.clone()).collect();
        model.graph_run = Some(GraphRunSheet {
            id: format!("bench-{count}"),
            selected_index: count / 2,
            selected_node_id: Some(format!("n{:04}", count / 2)),
            tasks,
            ..Default::default()
        });
        let layout_time = measured(|| {
            layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                160,
                42,
            )
        });
        let layout = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            160,
            42,
        );
        assert_eq!(layout.nodes.len(), count);
        assert!(layout.issues.is_empty(), "{:?}", layout.issues);
        let render_time = measured(|| graph_run::lines_with_layout(&model, 48, &layout));
        let target_id = format!("n{:04}", count / 2);
        select(
            model.graph_run.as_mut().unwrap(),
            &mut model.graph_canvas,
            &layout,
            &target_id,
        );
        let offset = viewport(
            &layout,
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
        );
        let target = layout
            .nodes
            .iter()
            .find(|node| node.id == target_id)
            .unwrap();
        let column = target.rect.x as i32 - offset.0 + 1;
        let row = target.rect.y as i32 - offset.1 + 1;
        assert!((0..layout.canvas.width as i32).contains(&column));
        assert!((0..layout.canvas.height as i32).contains(&row));
        let frame = GraphFrame {
            layout: layout.clone(),
            origin_y: 0,
            offset,
        };
        let hit_time = measured(|| {
            model.graph_run.as_mut().unwrap().selected_node_id = Some("n0000".into());
            assert!(handle_mouse(
                &mut model,
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: column as u16,
                    row: row as u16,
                    modifiers: KeyModifiers::NONE
                },
                &frame
            ));
            assert_eq!(
                model
                    .graph_run
                    .as_ref()
                    .unwrap()
                    .selected_node_id
                    .as_deref(),
                Some(target_id.as_str())
            );
        });
        let mut direction = [
            NavDirection::Right,
            NavDirection::Down,
            NavDirection::Left,
            NavDirection::Up,
        ]
        .into_iter()
        .cycle();
        let key_frame_time = measured(|| {
            let before = layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                160,
                42,
            );
            navigate(
                model.graph_run.as_mut().unwrap(),
                &mut model.graph_canvas,
                &before,
                direction.next().unwrap(),
            );
            let after = layout_graph(
                model.graph_run.as_ref().unwrap(),
                &model.graph_canvas,
                160,
                42,
            );
            graph_run::lines_with_layout(&model, 48, &after)
        });
        let rendered = graph_run::lines_with_layout(&model, 48, &layout);
        let render_text_bytes: usize = rendered
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.len())
            .sum();
        eprintln!(
            "{}",
            serde_json::json!({"fixture":"graph_dag","nodes":count,"layout":layout_time,
            "render":render_time,"key_to_frame_simulated":key_frame_time,"hit_test":hit_time,
            "layout_owned_bytes_estimate":owned_bytes(&layout),"render_text_bytes":render_text_bytes,
            "edges":layout.edges.len(),"edge_points":layout.edges.iter().map(|e|e.points.len()).sum::<usize>()})
        );
    }
}
