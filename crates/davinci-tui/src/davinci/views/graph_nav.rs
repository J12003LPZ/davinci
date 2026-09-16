//! Presentation-only navigation, never graph-controller commands.
use super::graph_layout::GraphLayout;
use crate::davinci::model::{GraphCanvasState, GraphRunSheet};
use crate::davinci::theme::State;

#[derive(Debug, Clone)]
pub struct GraphFrame {
    pub layout: GraphLayout,
    pub origin_y: u16,
    pub offset: (i32, i32),
}

pub fn handle_mouse(
    model: &mut crate::davinci::model::Model,
    mouse: crossterm::event::MouseEvent,
    frame: &GraphFrame,
) -> bool {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    if model.screen != crate::davinci::model::Screen::GraphRun
        || model.overlay.is_some()
        || model.voice.setup
    {
        return false;
    }
    let y = mouse.row as i32 - frame.origin_y as i32;
    if y < 0 || mouse.column >= frame.layout.canvas.width || y >= frame.layout.canvas.height as i32
    {
        return false;
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let point = (mouse.column as i32 + frame.offset.0, y + frame.offset.1);
            if let Some(node) = frame.layout.nodes.iter().find(|n| {
                point.0 >= n.rect.x as i32
                    && point.0 < n.rect.right() as i32
                    && point.1 >= n.rect.y as i32
                    && point.1 < n.rect.bottom() as i32
            }) {
                if let Some(run) = &mut model.graph_run {
                    select(run, &mut model.graph_canvas, &frame.layout, &node.id);
                }
            }
            true
        }
        MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => {
            let amount = if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft
            ) {
                -3
            } else {
                3
            };
            let horizontal = mouse.modifiers.contains(KeyModifiers::SHIFT)
                || matches!(
                    mouse.kind,
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                );
            pan(
                model,
                &frame.layout,
                if horizontal { amount } else { 0 },
                if horizontal { 0 } else { amount },
            );
            true
        }
        _ => false,
    }
}

pub fn pan(model: &mut crate::davinci::model::Model, layout: &GraphLayout, dx: i32, dy: i32) {
    if let Some(run) = &model.graph_run {
        let (x, y) = viewport(layout, run, &model.graph_canvas);
        model.graph_canvas.follow_live = false;
        model.graph_canvas.pan_x = x.saturating_add(dx).clamp(
            0,
            layout.content_width.saturating_sub(layout.canvas.width) as i32,
        );
        model.graph_canvas.pan_y = y.saturating_add(dy).clamp(
            0,
            layout.content_height.saturating_sub(layout.canvas.height) as i32,
        );
    }
}

pub fn handle_key(
    model: &mut crate::davinci::model::Model,
    key: crossterm::event::KeyEvent,
) -> bool {
    use crate::davinci::model::GraphViewMode;
    use crossterm::event::{KeyCode, KeyEventKind};
    if !key.modifiers.is_empty() || key.kind == KeyEventKind::Release {
        return false;
    }
    let relevant = matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Char('f' | 'v' | 'x' | 'r' | 'd')
    );
    if !relevant || model.graph_run.is_none() {
        return false;
    }
    let frame = crate::davinci::app::compose_frame(model, model.height).graph;
    let layout = frame
        .map(|f| f.layout)
        .or_else(|| super::graph_run::layout_for(model, model.height.saturating_sub(3)))
        .unwrap();
    match key.code {
        KeyCode::Char('f') => {
            model.graph_canvas.follow_live = true;
            model.feature_scroll = 0;
        }
        KeyCode::Char('v') => {
            model.graph_canvas.view_mode = match model.graph_canvas.view_mode {
                GraphViewMode::Overview => GraphViewMode::Focus,
                GraphViewMode::Focus => GraphViewMode::Overview,
            };
        }
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
            let direction = match key.code {
                KeyCode::Up => NavDirection::Up,
                KeyCode::Down => NavDirection::Down,
                KeyCode::Left => NavDirection::Left,
                _ => NavDirection::Right,
            };
            navigate(
                model.graph_run.as_mut().unwrap(),
                &mut model.graph_canvas,
                &layout,
                direction,
            );
            model.feature_scroll = 0;
        }
        KeyCode::Enter => {
            if let Some(group) = model.graph_canvas.selected_group.take() {
                toggle_group(&mut model.graph_canvas, &group);
                if let Some(node) = layout.nodes.iter().find(|n| n.id == group) {
                    let run = model.graph_run.as_mut().unwrap();
                    run.selected_node_id = node.members.first().cloned();
                    run.selected_index = node.task_index;
                }
            } else {
                let run = model.graph_run.as_mut().unwrap();
                if run.selected_node_id.is_none() {
                    if let Some(node) = layout.nodes.iter().find(|n| n.members.len() == 1) {
                        select(run, &mut model.graph_canvas, &layout, &node.id);
                    }
                }
                run.inspecting_node = !run.inspecting_node;
            }
            model.graph_canvas.inspector_scroll = 0;
        }
        KeyCode::Esc => {
            let run = model.graph_run.as_mut().unwrap();
            if run.inspecting_node || run.showing_diff {
                run.inspecting_node = false;
                run.showing_diff = false;
            } else if !model.graph_canvas.expanded_groups.is_empty() {
                model.graph_canvas.expanded_groups.clear();
            } else {
                return false;
            }
        }
        KeyCode::PageUp | KeyCode::PageDown => {
            let amount = if key.code == KeyCode::PageUp { -5 } else { 5 };
            if model.graph_run.as_ref().unwrap().inspecting_node {
                model.graph_canvas.inspector_scroll = model
                    .graph_canvas
                    .inspector_scroll
                    .saturating_add_signed(amount);
            } else if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
                model.feature_scroll = model.feature_scroll.saturating_add_signed(amount);
            } else {
                pan(model, &layout, 0, amount as i32);
            }
            model.graph_canvas.follow_live = false;
        }
        KeyCode::Char('x' | 'r' | 'd') if model.graph_canvas.selected_group.is_some() => {
            model.section_notice = Some("Expand the group and select a worker first".into());
        }
        _ => return false,
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

pub fn move_selection(
    layout: &GraphLayout,
    selected: Option<&str>,
    direction: NavDirection,
) -> Option<String> {
    let Some((index, current)) = layout
        .nodes
        .iter()
        .enumerate()
        .find(|(_, n)| Some(n.id.as_str()) == selected)
    else {
        return layout.nodes.first().map(|n| n.id.clone());
    };
    let semantic: Vec<_> = layout
        .edges
        .iter()
        .filter_map(|e| match direction {
            NavDirection::Left if e.to == index => Some(e.from),
            NavDirection::Right if e.from == index => Some(e.to),
            _ => None,
        })
        .collect();
    layout
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            if !semantic.is_empty() {
                return semantic.contains(i);
            }
            match direction {
                NavDirection::Left => n.depth < current.depth,
                NavDirection::Right => n.depth > current.depth,
                NavDirection::Up => n.depth == current.depth && n.rect.y < current.rect.y,
                NavDirection::Down => n.depth == current.depth && n.rect.y > current.rect.y,
            }
        })
        .min_by_key(|(_, n)| {
            (
                n.rect.x.abs_diff(current.rect.x) as u32 + n.rect.y.abs_diff(current.rect.y) as u32,
                n.task_index,
                &n.id,
            )
        })
        .map(|(_, n)| n.id.clone())
        .or_else(|| Some(current.id.clone()))
}

/// Fit the active bounding region when it fits; otherwise use the first active
/// task in stable task order. This never changes selection or execution state.
pub fn viewport(
    layout: &GraphLayout,
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
) -> (i32, i32) {
    let clamp = |x: i32, y: i32| {
        (
            x.clamp(
                0,
                layout.content_width.saturating_sub(layout.canvas.width) as i32,
            ),
            y.clamp(
                0,
                layout.content_height.saturating_sub(layout.canvas.height) as i32,
            ),
        )
    };
    if !canvas.follow_live {
        return clamp(canvas.pan_x, canvas.pan_y);
    }
    let active: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| run.tasks[n.task_index].state == State::Active)
        .collect();
    let Some(first) = active.first().copied().or_else(|| {
        layout
            .nodes
            .iter()
            .find(|n| Some(n.id.as_str()) == run.selected_node_id.as_deref())
    }) else {
        return clamp(0, 0);
    };
    let bounds = active.iter().fold(first.rect, |rect, n| rect.union(n.rect));
    let target = if bounds.width <= layout.canvas.width && bounds.height <= layout.canvas.height {
        bounds
    } else {
        first.rect
    };
    clamp(
        target.x as i32 - layout.canvas.width.saturating_sub(target.width) as i32 / 2,
        target.y as i32 - layout.canvas.height.saturating_sub(target.height) as i32 / 2,
    )
}

pub fn select(
    run: &mut GraphRunSheet,
    canvas: &mut GraphCanvasState,
    layout: &GraphLayout,
    id: &str,
) {
    let Some(node) = layout.nodes.iter().find(|n| n.id == id) else {
        return;
    };
    let (x, y) = viewport(layout, run, canvas);
    canvas.follow_live = false;
    canvas.selected_group = (node.members.len() > 1).then(|| id.into());
    run.selected_node_id = (node.members.len() == 1).then(|| id.into());
    run.selected_index = node.task_index;
    canvas.inspector_scroll = 0;
    canvas.pan_x = x
        .min(node.rect.x as i32)
        .max(node.rect.right() as i32 - layout.canvas.width as i32)
        .max(0);
    canvas.pan_y = y
        .min(node.rect.y as i32)
        .max(node.rect.bottom() as i32 - layout.canvas.height as i32)
        .max(0);
}

pub fn navigate(
    run: &mut GraphRunSheet,
    canvas: &mut GraphCanvasState,
    layout: &GraphLayout,
    direction: NavDirection,
) {
    let selected = canvas
        .selected_group
        .as_deref()
        .or(run.selected_node_id.as_deref());
    if let Some(id) = move_selection(layout, selected, direction) {
        select(run, canvas, layout, &id);
    }
    canvas.follow_live = false;
}

pub fn toggle_group(canvas: &mut GraphCanvasState, id: &str) {
    if !canvas.expanded_groups.remove(id) {
        canvas.expanded_groups.insert(id.into());
    }
}

pub fn focus_nodes(
    layout: &GraphLayout,
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
) -> std::collections::BTreeSet<String> {
    let selected = canvas
        .selected_group
        .as_deref()
        .or(run.selected_node_id.as_deref());
    let seeds: Vec<_> = layout
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            selected.map_or(run.tasks[n.task_index].state == State::Active, |id| {
                id == n.id
            })
        })
        .map(|(i, _)| i)
        .collect();
    let mut result = std::collections::BTreeSet::new();
    for reverse in [false, true] {
        let mut links = vec![Vec::new(); layout.nodes.len()];
        for e in &layout.edges {
            if reverse {
                links[e.to].push(e.from);
            } else {
                links[e.from].push(e.to);
            }
        }
        let mut pending = seeds.clone();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(i) = pending.pop() {
            if seen.insert(i) {
                result.insert(layout.nodes[i].id.clone());
                pending.extend(&links[i]);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::super::graph_layout::layout_graph;
    use super::*;
    use crate::davinci::{fixtures, theme::State};

    #[test]
    fn graph_nav_semantic_neighbors_and_follow() {
        let mut run = fixtures::blueprint_graph();
        let mut canvas = GraphCanvasState::default();
        let layout = layout_graph(&run, &canvas, 120, 30);
        assert_eq!(
            move_selection(&layout, Some("writer"), NavDirection::Left).as_deref(),
            Some("plan")
        );
        assert_eq!(
            move_selection(&layout, Some("writer"), NavDirection::Down).as_deref(),
            Some("tests")
        );
        navigate(&mut run, &mut canvas, &layout, NavDirection::Right);
        assert!(!canvas.follow_live);
        canvas.follow_live = true;
        assert!(viewport(&layout, &run, &canvas).0 > 0);
    }

    #[test]
    fn graph_nav_folds_only_quiet_compatible_frontiers_and_expands_reversibly() {
        let mut run = fixtures::blueprint_graph();
        let mut canvas = GraphCanvasState::default();
        let layout = layout_graph(&run, &canvas, 120, 30);
        let group = layout
            .nodes
            .iter()
            .find(|n| n.members.len() == 3)
            .expect("three completed researchers fold");
        let group_id = group.id.clone();
        toggle_group(&mut canvas, &group_id);
        assert!(layout_graph(&run, &canvas, 120, 30)
            .nodes
            .iter()
            .all(|n| n.members.len() == 1));
        run.tasks
            .iter_mut()
            .find(|t| t.id == "writer")
            .unwrap()
            .state = State::Done;
        assert!(layout_graph(&run, &canvas, 120, 30)
            .nodes
            .iter()
            .all(|n| n.members.len() == 1));
        for state in [
            State::Active,
            State::Failed,
            State::Attention,
            State::Queued,
        ] {
            let mut run = fixtures::blueprint_graph();
            run.tasks[1].state = state;
            assert!(layout_graph(&run, &GraphCanvasState::default(), 120, 30)
                .nodes
                .iter()
                .any(|n| n.id == "research-a" && n.members.len() == 1));
        }
        run = fixtures::blueprint_graph();
        run.selected_node_id = Some("research-a".into());
        assert!(layout_graph(&run, &GraphCanvasState::default(), 120, 30)
            .nodes
            .iter()
            .any(|n| n.id == "research-a"));
        run.selected_node_id = None;
        run.tasks[1].public_contract = Some("verification pending".into());
        assert!(layout_graph(&run, &GraphCanvasState::default(), 120, 30)
            .nodes
            .iter()
            .any(|n| n.id == "research-a"));
    }
}
