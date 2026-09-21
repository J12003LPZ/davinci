//! Pure terminal-cell geometry. The renderer and input share these rectangles.
use crate::davinci::model::{GraphCanvasState, GraphRunSheet};
use ratatui::layout::Rect;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphResponsiveMode {
    Full,
    Adaptive,
    Compact,
    Structured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    pub id: String,
    pub members: Vec<String>,
    pub task_index: usize,
    pub depth: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutEdge {
    pub from: usize,
    pub to: usize,
    pub points: Vec<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLayout {
    pub mode: GraphResponsiveMode,
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub canvas: Rect,
    pub inspector: Rect,
    pub content_width: u16,
    pub content_height: u16,
    pub issues: Vec<String>,
}

pub fn layout_graph(
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
    width: u16,
    height: u16,
) -> GraphLayout {
    let mode = match (width, height) {
        (0..=49, _) | (_, 0..=11) => GraphResponsiveMode::Structured,
        (100.., 16..) => GraphResponsiveMode::Full,
        (72.., _) => GraphResponsiveMode::Adaptive,
        _ => GraphResponsiveMode::Compact,
    };
    let inspector = match mode {
        GraphResponsiveMode::Full => {
            let panel_width = (width / 3).clamp(36, 72);
            Rect::new(width - panel_width, 0, panel_width, height)
        }
        GraphResponsiveMode::Adaptive | GraphResponsiveMode::Compact => {
            let drawer = (height / 3).clamp(4, 9);
            Rect::new(0, height - drawer, width, drawer)
        }
        GraphResponsiveMode::Structured => Rect::default(),
    };
    let mut layout = GraphLayout {
        mode,
        nodes: vec![],
        edges: vec![],
        canvas: match mode {
            GraphResponsiveMode::Full => Rect::new(0, 0, inspector.x.saturating_sub(1), height),
            GraphResponsiveMode::Adaptive | GraphResponsiveMode::Compact => {
                Rect::new(0, 0, width, inspector.y.saturating_sub(1))
            }
            GraphResponsiveMode::Structured => Rect::new(0, 0, width, height),
        },
        inspector,
        content_width: 0,
        content_height: 0,
        issues: vec![],
    };
    let ids: BTreeMap<&str, usize> = run
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    if ids.len() != run.tasks.len() {
        layout.mode = GraphResponsiveMode::Structured;
        layout
            .issues
            .push("layout unavailable: duplicate node IDs".into());
        return layout;
    }
    let mut children = vec![Vec::new(); run.tasks.len()];
    let mut pending = vec![0usize; run.tasks.len()];
    for (i, task) in run.tasks.iter().enumerate() {
        for dep in task.dependencies.iter().collect::<BTreeSet<_>>() {
            if let Some(&parent) = ids.get(dep.as_str()) {
                children[parent].push(i);
                pending[i] += 1;
            } else {
                layout
                    .issues
                    .push(format!("{}: unresolved dependency {dep}", task.id));
            }
        }
    }
    let mut ready: BTreeSet<usize> = pending
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n == 0).then_some(i))
        .collect();
    let mut depths = vec![0usize; run.tasks.len()];
    let mut visited = 0;
    while let Some(i) = ready.pop_first() {
        visited += 1;
        for &child in &children[i] {
            depths[child] = depths[child].max(depths[i] + 1);
            pending[child] -= 1;
            if pending[child] == 0 {
                ready.insert(child);
            }
        }
    }
    if visited != run.tasks.len() {
        layout.mode = GraphResponsiveMode::Structured;
        layout
            .issues
            .push("layout unavailable: cyclic dependencies".into());
        return layout;
    }
    let card_width = if mode == GraphResponsiveMode::Compact {
        20
    } else {
        26
    };
    let card_height = if mode == GraphResponsiveMode::Compact {
        4
    } else {
        5
    };
    let mut occupied = BTreeMap::<usize, BTreeMap<usize, usize>>::new();
    let order: BTreeMap<_, _> = canvas
        .node_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut peers: Vec<_> = (0..run.tasks.len()).collect();
    peers.sort_by_key(|&i| {
        (
            depths[i],
            order
                .get(run.tasks[i].id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
            i,
        )
    });
    let mut slots = vec![0; run.tasks.len()];
    for i in peers {
        // Align a child with a declared parent when the lane has room. First-
        // seen peers reserve their slots first, so newly appended workers do
        // not displace them. Path compression avoids quadratic sibling scans.
        let preferred = run.tasks[i]
            .dependencies
            .iter()
            .filter_map(|id| ids.get(id.as_str()).map(|&parent| slots[parent]))
            .min()
            .unwrap_or(0);
        let lane = occupied.entry(depths[i]).or_default();
        let mut row = preferred;
        let mut path = Vec::new();
        while let Some(&next) = lane.get(&row) {
            path.push(row);
            row = next;
        }
        for visited in path {
            lane.insert(visited, row + 1);
        }
        lane.insert(row, row + 1);
        slots[i] = row;
    }
    for (i, task) in run.tasks.iter().enumerate() {
        let depth = depths[i];
        let x = depth.saturating_mul(card_width + 6);
        let y = slots[i].saturating_mul(card_height + 2).saturating_add(2);
        if x + card_width > u16::MAX as usize || y + card_height > u16::MAX as usize {
            layout.mode = GraphResponsiveMode::Structured;
            layout
                .issues
                .push("layout unavailable: graph exceeds cell coordinate range".into());
            layout.nodes.clear();
            return layout;
        }
        let rect = Rect::new(x as u16, y as u16, card_width as u16, card_height as u16);
        layout.content_width = layout.content_width.max(rect.right());
        layout.content_height = layout.content_height.max(rect.bottom());
        layout.nodes.push(LayoutNode {
            id: task.id.clone(),
            members: vec![task.id.clone()],
            task_index: i,
            depth,
            rect,
        });
    }
    for (from, next) in children.iter().enumerate() {
        for &to in next {
            layout.edges.push(LayoutEdge {
                from,
                to,
                points: Vec::new(),
            });
        }
    }
    fold_completed(layout, run, canvas)
}

fn fold_completed(
    mut layout: GraphLayout,
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
) -> GraphLayout {
    use crate::davinci::theme::State;
    // Equal incoming AND outgoing frontiers ensure a summary never invents
    // connectivity between a member and a different member's dependent.
    let mut outgoing = vec![Vec::new(); run.tasks.len()];
    for edge in &layout.edges {
        outgoing[edge.from].push(edge.to);
    }
    let mut groups = BTreeMap::new();
    let real_ids: BTreeSet<_> = run.tasks.iter().map(|t| t.id.as_str()).collect();
    for node in &layout.nodes {
        let task = &run.tasks[node.task_index];
        if task.state != State::Done
            || task.error.is_some()
            || task.public_contract.is_some()
            || task.role.is_empty()
            || task.role.contains("verif")
            || task.role.contains("review")
            || matches!(task.phase.as_str(), "verify" | "review")
            || task
                .dependencies
                .iter()
                .any(|id| !real_ids.contains(id.as_str()))
            || run.selected_node_id.as_deref() == Some(&task.id)
        {
            continue;
        }
        let incoming: BTreeSet<_> = task.dependencies.iter().collect();
        groups
            .entry((
                node.depth,
                &task.role,
                &task.phase,
                incoming,
                &outgoing[node.task_index],
            ))
            .or_insert_with(Vec::new)
            .push(node.task_index);
    }
    let mut membership = BTreeMap::new();
    let mut folded = BTreeMap::new();
    for members in groups.values_mut().filter(|g| g.len() >= 3) {
        members.sort_by_key(|&i| layout.nodes[i].rect.y);
        let first = members[0];
        let id = format!("fold:{}", run.tasks[first].id);
        if canvas.expanded_groups.contains(&id) || real_ids.contains(id.as_str()) {
            continue;
        }
        for &member in members.iter() {
            membership.insert(member, first);
        }
        folded.insert(
            first,
            (
                id,
                members
                    .iter()
                    .map(|&i| run.tasks[i].id.clone())
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let mut nodes = Vec::new();
    let mut remap = BTreeMap::new();
    for node in &layout.nodes {
        let owner = membership
            .get(&node.task_index)
            .copied()
            .unwrap_or(node.task_index);
        if owner != node.task_index {
            continue;
        }
        remap.insert(owner, nodes.len());
        let mut node = node.clone();
        if let Some((id, members)) = folded.get(&owner) {
            node.id = id.clone();
            node.members = members.clone();
        }
        nodes.push(node);
    }
    let mut pairs = BTreeSet::new();
    for edge in &layout.edges {
        let from = membership.get(&edge.from).copied().unwrap_or(edge.from);
        let to = membership.get(&edge.to).copied().unwrap_or(edge.to);
        pairs.insert((remap[&from], remap[&to]));
    }
    layout.edges = pairs
        .into_iter()
        .map(|(from, to)| {
            let a = nodes[from].rect;
            let b = nodes[to].rect;
            let start = (a.right(), a.y + a.height / 2);
            let end = (b.x - 1, b.y + b.height / 2);
            // Independent destinations use separate corridor tracks. Equal
            // destinations may share a join; unrelated joins must not become
            // one apparent vertical bus. Coordinates keep routing stable.
            let mid = a.right() + 1 + (b.y / (b.height + 2)) % 4;
            let points = if nodes[to].depth > nodes[from].depth + 1 {
                vec![
                    start,
                    (mid, start.1),
                    (mid, 1),
                    (end.0 - 1, 1),
                    (end.0 - 1, end.1),
                    end,
                ]
            } else if start.1 == end.1 {
                vec![start, end]
            } else {
                vec![start, (mid, start.1), (mid, end.1), end]
            };
            LayoutEdge { from, to, points }
        })
        .collect();
    layout.content_height = nodes.iter().map(|n| n.rect.bottom()).max().unwrap_or(0);
    layout.nodes = nodes;
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::GraphTask;

    pub fn diamond() -> GraphRunSheet {
        GraphRunSheet {
            tasks: [
                ("start", vec![]),
                ("left", vec!["start"]),
                ("right", vec!["start"]),
                ("join", vec!["left", "right"]),
            ]
            .into_iter()
            .map(|(id, deps)| GraphTask {
                id: id.into(),
                dependencies: deps.into_iter().map(str::to_owned).collect(),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn graph_layout_deterministic_dependency_order_and_selection() {
        let run = GraphRunSheet {
            selected_node_id: Some("right".into()),
            ..diamond()
        };
        let canvas = GraphCanvasState::default();
        let layout = layout_graph(&run, &canvas, 120, 30);
        assert_eq!(layout.nodes.len(), 4);
        assert_eq!(layout.edges.len(), 4);
        assert_eq!(layout, layout_graph(&run, &canvas, 120, 30));
        let ids: std::collections::BTreeSet<_> =
            layout.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids.len(), 4);
        assert!(ids.contains("right"));
        for edge in &layout.edges {
            assert!(layout.nodes[edge.from].rect.right() < layout.nodes[edge.to].rect.x);
        }
    }

    #[test]
    fn graph_layout_responsive_and_bounded_inspector() {
        for (width, height, mode) in [
            (40, 30, GraphResponsiveMode::Structured),
            (50, 30, GraphResponsiveMode::Compact),
            (80, 30, GraphResponsiveMode::Adaptive),
            (120, 30, GraphResponsiveMode::Full),
            (120, 21, GraphResponsiveMode::Full),
            (109, 21, GraphResponsiveMode::Full),
            (120, 5, GraphResponsiveMode::Structured),
        ] {
            let layout = layout_graph(&diamond(), &GraphCanvasState::default(), width, height);
            assert_eq!(layout.mode, mode);
            assert!(layout.inspector.right() <= width);
            assert!(layout.inspector.bottom() <= height);
        }
    }

    #[test]
    fn graph_layout_separates_unrelated_join_routes() {
        let run = crate::davinci::fixtures::blueprint_graph();
        let layout = layout_graph(&run, &GraphCanvasState::default(), 120, 34);
        let buses = |id: &str| {
            layout
                .edges
                .iter()
                .filter(|e| layout.nodes[e.to].id == id)
                .flat_map(|e| e.points.windows(2))
                .filter(|p| p[0].0 == p[1].0 && p[0].1 != p[1].1)
                .map(|p| p[0].0)
                .collect::<BTreeSet<_>>()
        };
        assert!(buses("blocked").is_disjoint(&buses("review")));
    }

    #[test]
    fn graph_layout_aligns_declared_branches_without_false_horizontal_paths() {
        let run = crate::davinci::fixtures::blueprint_graph();
        let layout = layout_graph(&run, &GraphCanvasState::default(), 120, 34);
        let y = |id: &str| layout.nodes.iter().find(|n| n.id == id).unwrap().rect.y;
        assert_eq!(y("failure"), y("blocked"));
        assert_eq!(y("writer"), y("review"));
        assert_ne!(y("writer"), y("blocked"));
    }

    #[test]
    fn graph_layout_rejects_cycles_duplicates_and_marks_missing_edges() {
        let mut run = diamond();
        run.tasks[0].dependencies.push("join".into());
        let cycle = layout_graph(&run, &GraphCanvasState::default(), 120, 30);
        assert_eq!(cycle.mode, GraphResponsiveMode::Structured);
        assert!(!cycle.issues.is_empty());
        run.tasks[0].dependencies = vec!["missing".into()];
        let missing = layout_graph(&run, &GraphCanvasState::default(), 120, 30);
        assert_eq!(missing.nodes.len(), 4);
        assert!(missing.issues.iter().any(|s| s.contains("missing")));
        run.tasks[1].id = "start".into();
        assert_eq!(
            layout_graph(&run, &GraphCanvasState::default(), 120, 30).mode,
            GraphResponsiveMode::Structured
        );
    }
}
