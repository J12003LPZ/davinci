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

/// Replace a public snapshot without turning refresh into a navigation event.
pub fn refresh(model: &mut crate::davinci::model::Model, mut next: GraphRunSheet) {
    if let Some(previous) = model.graph_run.as_ref().filter(|p| p.id == next.id) {
        next.inspecting_node = previous.inspecting_node;
        next.showing_diff = previous.showing_diff;
        next.selected_node_id = previous
            .selected_node_id
            .as_deref()
            .and_then(|id| nearest_survivor(previous, &next, id));
        next.selected_index = next
            .selected_node_id
            .as_deref()
            .and_then(|id| next.tasks.iter().position(|t| t.id == id))
            .unwrap_or(0);
    } else {
        model.graph_canvas = GraphCanvasState::default();
        model.feature_scroll = 0;
    }
    let mut known: std::collections::BTreeSet<_> =
        model.graph_canvas.node_order.iter().cloned().collect();
    for task in &next.tasks {
        if known.insert(task.id.clone()) {
            model.graph_canvas.node_order.push(task.id.clone());
        }
    }
    model.graph_run = Some(next);
    // A summary can disappear when a member becomes attention-requiring. Never
    // keep an invisible synthetic selection or infer a controller target.
    if let Some(group) = &model.graph_canvas.selected_group {
        let run = model.graph_run.as_ref().unwrap();
        let layout =
            super::graph_layout::layout_graph(run, &model.graph_canvas, model.width, model.height);
        if !layout.nodes.iter().any(|n| &n.id == group) {
            model.graph_canvas.selected_group = None;
        }
    }
}

fn nearest_survivor(
    previous: &GraphRunSheet,
    next: &GraphRunSheet,
    selected: &str,
) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    let survivors: BTreeSet<_> = next.tasks.iter().map(|t| t.id.as_str()).collect();
    if survivors.contains(selected) {
        return Some(selected.into());
    }
    let mut neighbors = BTreeMap::<&str, Vec<&str>>::new();
    for task in &previous.tasks {
        neighbors
            .entry(&task.id)
            .or_default()
            .extend(task.dependencies.iter().map(String::as_str));
    }
    for task in &previous.tasks {
        for dep in &task.dependencies {
            neighbors.entry(dep).or_default().push(&task.id);
        }
    }
    let mut seen = BTreeSet::from([selected]);
    let mut queue = VecDeque::from([selected]);
    while let Some(id) = queue.pop_front() {
        for &neighbor in neighbors.get(id).into_iter().flatten() {
            if !seen.insert(neighbor) {
                continue;
            }
            if survivors.contains(neighbor) {
                return Some(neighbor.into());
            }
            queue.push_back(neighbor);
        }
    }
    None
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
        || frame.layout.mode == super::graph_layout::GraphResponsiveMode::Structured
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
            model.graph_canvas.list_scroll = None;
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
            model.graph_canvas.list_scroll = None;
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
                    navigate(run, &mut model.graph_canvas, &layout, NavDirection::Down);
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
                let (width, height) =
                    if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
                        (model.width, layout.canvas.height.saturating_sub(3))
                    } else {
                        (layout.inspector.width, layout.inspector.height)
                    };
                super::graph_inspector::page(model, width, height, amount);
            } else if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
                let run = model.graph_run.as_ref().unwrap();
                let start = model.graph_canvas.list_scroll.unwrap_or(run.selected_index);
                model.graph_canvas.list_scroll = Some(
                    start
                        .saturating_add_signed(amount)
                        .min(run.tasks.len().saturating_sub(1)),
                );
            } else {
                pan(model, &layout, 0, amount as i32);
            }
            model.graph_canvas.follow_live = false;
        }
        KeyCode::Char('x' | 'r' | 'd') if model.graph_canvas.selected_group.is_some() => {
            model.section_notice = Some("Select a real worker first (Enter expands groups)".into());
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
    let Some(first) = active
        .iter()
        .copied()
        .min_by_key(|n| (n.rect.x, n.rect.y))
        .or_else(|| {
            layout
                .nodes
                .iter()
                .find(|n| Some(n.id.as_str()) == run.selected_node_id.as_deref())
        })
    else {
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
    if layout.mode == super::graph_layout::GraphResponsiveMode::Structured {
        let count = run.tasks.len();
        if count > 0 {
            let index = run
                .selected_node_id
                .as_deref()
                .and_then(|id| run.tasks.iter().position(|t| t.id == id));
            let next = match (index, direction) {
                (Some(i), NavDirection::Up | NavDirection::Left) => i.saturating_sub(1),
                (Some(i), _) => (i + 1).min(count - 1),
                _ => 0,
            };
            run.selected_node_id = Some(run.tasks[next].id.clone());
            run.selected_index = next;
            canvas.selected_group = None;
            canvas.inspector_scroll = 0;
        }
        canvas.follow_live = false;
        return;
    }
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
    fn graph_refresh_preserves_selection_fold_follow_and_peer_positions() {
        use crate::davinci::{
            model::Model,
            theme::{ColorDepth, Theme},
        };
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 120, 40, false);
        refresh(&mut model, fixtures::blueprint_graph());
        model.graph_run.as_mut().unwrap().selected_node_id = Some("writer".into());
        model.graph_run.as_mut().unwrap().inspecting_node = true;
        model.graph_canvas.follow_live = false;
        model.graph_canvas.pan_x = 7;
        let old = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            120,
            30,
        );
        let mut next = fixtures::blueprint_graph();
        next.tasks[5].state = State::Done;
        next.tasks[6].state = State::Failed;
        let mut added = next.tasks[5].clone();
        added.id = "new-worker".into();
        next.tasks.insert(0, added);
        next.tasks.reverse();
        refresh(&mut model, next);
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("writer")
        );
        assert!(model.graph_run.as_ref().unwrap().inspecting_node);
        assert!(!model.graph_canvas.follow_live);
        assert_eq!(model.graph_canvas.pan_x, 7);
        let new = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            120,
            30,
        );
        for node in &old.nodes {
            assert_eq!(
                new.nodes.iter().find(|n| n.id == node.id).unwrap().rect,
                node.rect,
                "{} moved",
                node.id
            );
        }
        let group = old
            .nodes
            .iter()
            .find(|n| n.members.len() > 1)
            .unwrap()
            .id
            .clone();
        model.graph_run.as_mut().unwrap().selected_node_id = None;
        model.graph_canvas.selected_group = Some(group.clone());
        let next = model.graph_run.as_ref().unwrap().clone();
        refresh(&mut model, next);
        assert!(model.graph_run.as_ref().unwrap().selected_node_id.is_none());
        assert_eq!(model.graph_canvas.selected_group.as_ref(), Some(&group));
    }

    #[test]
    fn graph_refresh_disappearance_uses_real_neighbor_and_new_run_resets_view() {
        use crate::davinci::{
            model::Model,
            theme::{ColorDepth, Theme},
        };
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 80, 32, false);
        refresh(&mut model, fixtures::blueprint_graph());
        model.graph_run.as_mut().unwrap().selected_node_id = Some("writer".into());
        let mut next = fixtures::blueprint_graph();
        next.tasks.retain(|t| t.id != "writer");
        refresh(&mut model, next.clone());
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("plan")
        );
        next.tasks.clear();
        refresh(&mut model, next);
        assert!(model.graph_run.as_ref().unwrap().selected_node_id.is_none());
        model.graph_canvas.follow_live = false;
        let mut next = fixtures::blueprint_graph();
        next.id = "another-run".into();
        refresh(&mut model, next);
        assert!(model.graph_canvas.follow_live);
    }

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
    fn graph_fallback_keys_and_paging_stay_bounded_even_with_a_cycle() {
        use crate::davinci::{
            app,
            model::{Model, Screen},
            theme::{ColorDepth, Theme},
        };
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 40, 32, false);
        model.screen = Screen::GraphRun;
        let mut run = fixtures::blueprint_graph();
        run.tasks[0].dependencies.push("review".into());
        refresh(&mut model, run);
        let press = |model: &mut Model, code| {
            app::handle_key(model, KeyEvent::new(code, KeyModifiers::NONE));
        };
        press(&mut model, KeyCode::Down);
        press(&mut model, KeyCode::Down);
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("research-a")
        );
        assert!(!model.graph_canvas.follow_live);
        for _ in 0..20 {
            press(&mut model, KeyCode::PageDown);
        }
        assert_eq!(model.graph_canvas.list_scroll, Some(9));
        press(&mut model, KeyCode::PageUp);
        assert_eq!(model.graph_canvas.list_scroll, Some(4));
        press(&mut model, KeyCode::Enter);
        for _ in 0..20 {
            press(&mut model, KeyCode::PageDown);
        }
        let bottom = model.graph_canvas.inspector_scroll;
        press(&mut model, KeyCode::PageUp);
        assert_eq!(
            model.graph_canvas.inspector_scroll,
            bottom.saturating_sub(5)
        );
        press(&mut model, KeyCode::Esc);
        assert!(!model.graph_run.as_ref().unwrap().inspecting_node);
        press(&mut model, KeyCode::Char('f'));
        assert!(model.graph_canvas.follow_live);
        assert!(model.graph_canvas.list_scroll.is_none());
    }

    #[test]
    fn graph_fold_controls_never_send_synthetic_node_and_attention_unfolds() {
        use crate::davinci::{
            app::{self, Flow},
            model::{Model, Screen},
            theme::{ColorDepth, Theme},
        };
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 120, 40, false);
        model.screen = Screen::GraphRun;
        refresh(&mut model, fixtures::blueprint_graph());
        let layout = layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            120,
            30,
        );
        let group = layout.nodes.iter().find(|n| n.members.len() > 1).unwrap();
        select(
            model.graph_run.as_mut().unwrap(),
            &mut model.graph_canvas,
            &layout,
            &group.id,
        );
        for ch in ['x', 'r', 'd'] {
            assert!(matches!(
                app::handle_key(
                    &mut model,
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
                ),
                Flow::Continue
            ));
        }
        app::handle_key(
            &mut model,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(model.graph_canvas.expanded_groups.contains(&group.id));
        assert_eq!(
            model
                .graph_run
                .as_ref()
                .unwrap()
                .selected_node_id
                .as_deref(),
            Some("research-a")
        );
        let mut next = fixtures::blueprint_graph();
        next.tasks[1].state = State::Failed;
        refresh(&mut model, next);
        assert!(layout_graph(
            model.graph_run.as_ref().unwrap(),
            &model.graph_canvas,
            120,
            30
        )
        .nodes
        .iter()
        .any(|n| n.id == "research-a"));
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

#[cfg(test)]
#[path = "graph_perf.rs"]
mod perf;
